//! Which release applies to *this* install, and are these the bytes it named.
//!
//! [`crate::feed`] answers "what did the pipeline publish". This module answers
//! the two questions that follow, and they are the two an updater gets wrong:
//!
//! 1. **Does this release apply here?** Not "is it the newest" — newest is easy.
//!    A release applies when it is newer *and* the channel matches *and* the OS
//!    is new enough *and* an artifact exists for this platform *and* the install
//!    falls inside the staged rollout. [`applicability`] answers that for one
//!    release and names its reason; [`choose`] walks the feed and returns the
//!    first one that applies.
//!
//! 2. **Is the file we downloaded the file the feed described?** [`Download`]
//!    hashes the bytes as they land and compares size, digest and signature in
//!    that order — cheapest check first, so a truncated 200 MB download is
//!    rejected without hashing it.
//!
//! ```
//! use silka_dist::feed::{Feed, Platform};
//! use silka_dist::update::{choose, Install};
//! use silka_dist::version::Version;
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
//!
//! let install = Install::new("dev.silka.dashboard", Version::new(1, 3, 0))
//!     .platform(Platform::MacosArm64);
//!
//! let offer = choose(&feed, &install).unwrap().expect("1.4.0 applies");
//! assert_eq!(offer.version(), &Version::new(1, 4, 0));
//! assert_eq!(offer.url(), "https://example.com/a.dmg");
//! ```
//!
//! # The line this module will not cross
//!
//! It computes digests; it does **not** verify signatures. [`SignatureVerifier`]
//! is a trait the application implements with a real cryptography crate, and the
//! reason is in the crate README: a hand-rolled Ed25519 in a UI framework, with
//! no third-party test vectors, is a routine that looks like security and is
//! not. What this module guarantees is that the verifier is *asked* — an
//! artifact whose `signature` field is missing is a failure, not a pass, the
//! moment a verifier exists.

use std::fmt;

use crate::feed::{Artifact, Delta, Feed, Platform, Release};
use crate::sha256::{sha256, Digest, Sha256};
use crate::version::Version;

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

/// Everything the decision needs to know about the copy that is running.
///
/// Built once at startup and handed to [`choose`]. Constructed the way the rest
/// of the framework builds values (REKOMENDASI §2.5): a constructor function
/// with the two fields that have no sensible default, then chained methods.
#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    app: String,
    version: Version,
    channel: String,
    platform: Platform,
    os_version: Option<Version>,
    bucket: u8,
    pre_release: bool,
    deltas: bool,
    skipped: Vec<Version>,
}

impl Install {
    /// The running application: its bundle identifier and its version.
    ///
    /// Defaults: channel `stable`, [`Platform::current`], no known OS version,
    /// bucket `0`, no pre-releases, no deltas, nothing skipped.
    ///
    /// Bucket `0` is deliberately the *most* eager value — a rollout of 1% lands
    /// on it. An application that never calls [`Install::identifier`] therefore
    /// behaves like an early adopter rather than like a machine that silently
    /// never updates, and one of those two failures is visible in testing.
    pub fn new(app: impl Into<String>, version: Version) -> Install {
        Install {
            app: app.into(),
            version,
            channel: String::from("stable"),
            platform: Platform::current(),
            os_version: None,
            bucket: 0,
            pre_release: false,
            deltas: false,
            skipped: Vec::new(),
        }
    }

    /// Which channel this install follows. Must match the feed's own `channel`.
    pub fn channel(mut self, channel: impl Into<String>) -> Install {
        self.channel = channel.into();
        self
    }

    /// Override the platform. Only tests and cross-checks should need this.
    pub fn platform(mut self, platform: Platform) -> Install {
        self.platform = platform;
        self
    }

    /// The operating system version this machine reports.
    ///
    /// Absent means "unknown", and unknown is *allowed* through every
    /// `minimum_os` floor — see [`crate::feed::MinimumOs::allows`] for why
    /// stranding is the worse failure.
    pub fn os_version(mut self, version: Version) -> Install {
        self.os_version = Some(version);
        self
    }

    /// Place this install in a staged-rollout bucket, `0..=99`.
    ///
    /// Values above 99 are clamped rather than rejected: a bucket is a bucket,
    /// and returning a `Result` here would push error handling into startup code
    /// that has nothing useful to do with it.
    pub fn bucket(mut self, bucket: u8) -> Install {
        self.bucket = bucket.min(99);
        self
    }

    /// Derive the rollout bucket from a stable per-install string.
    ///
    /// Pass something that survives restarts and differs between machines — a
    /// UUID written at first launch is the usual answer. **Not** a username, a
    /// hostname or a MAC address: the bucket travels to no server, but a value
    /// derived from personal data is one leak away from being personal data.
    pub fn identifier(mut self, identifier: &str) -> Install {
        self.bucket = bucket_for(identifier);
        self
    }

    /// Accept releases that carry a pre-release tag.
    ///
    /// Separate from the channel on purpose: a `beta` channel feed may still
    /// publish finished builds, and a `stable` install that opted into
    /// pre-releases is a tester, not a channel change.
    pub fn pre_release(mut self, allow: bool) -> Install {
        self.pre_release = allow;
        self
    }

    /// Accept delta patches.
    ///
    /// **Off by default**, and that is the honest default: applying a delta
    /// needs a patcher, this crate ships none, and handing an application a
    /// `.delta` URL it cannot apply is worse than handing it the full download.
    /// Turn it on in the build that has a patcher wired up.
    pub fn deltas(mut self, allow: bool) -> Install {
        self.deltas = allow;
        self
    }

    /// Record a version the user chose to skip.
    ///
    /// A skip is not a mute button: a release marked `mandatory` is offered
    /// anyway. Publishing a mandatory release is how "we shipped a build that
    /// corrupts documents" gets undone.
    pub fn skip(mut self, version: Version) -> Install {
        if !self.skipped.contains(&version) {
            self.skipped.push(version);
        }
        self
    }

    /// The bundle identifier.
    pub fn app_id(&self) -> &str {
        &self.app
    }

    /// The version currently running.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The channel this install follows.
    pub fn channel_name(&self) -> &str {
        &self.channel
    }

    /// The platform this install runs on.
    pub fn platform_name(&self) -> &Platform {
        &self.platform
    }

    /// The OS version, if the application could read one.
    pub fn os(&self) -> Option<&Version> {
        self.os_version.as_ref()
    }

    /// The rollout bucket, `0..=99`.
    pub fn bucket_index(&self) -> u8 {
        self.bucket
    }

    /// Whether pre-releases are accepted.
    pub fn takes_pre_release(&self) -> bool {
        self.pre_release
    }

    /// Whether delta patches are accepted.
    pub fn takes_deltas(&self) -> bool {
        self.deltas
    }

    /// The versions the user skipped.
    pub fn skipped(&self) -> &[Version] {
        &self.skipped
    }
}

/// A stable rollout bucket, `0..=99`, for one install identifier.
///
/// The mapping is SHA-256 truncated to 64 bits, taken modulo 100. Two properties
/// matter and both come from it being a *hash* rather than a random draw:
///
/// - the same identifier lands in the same bucket forever, so raising a rollout
///   from 10% to 25% never *removes* an update from someone already offered it;
/// - the buckets are spread evenly, so a 5% rollout is 5% of installs and not 5%
///   of whoever happened to check first.
///
/// ```
/// use silka_dist::update::bucket_for;
///
/// let bucket = bucket_for("2f1c4e1a-…-install-uuid");
/// assert!(bucket < 100);
/// assert_eq!(bucket, bucket_for("2f1c4e1a-…-install-uuid"), "stable per install");
/// ```
pub fn bucket_for(identifier: &str) -> u8 {
    let digest = sha256(identifier.as_bytes());
    let bytes = digest.as_bytes();
    let mut value = 0u64;
    for byte in bytes.iter().take(8) {
        value = (value << 8) | u64::from(*byte);
    }
    (value % 100) as u8
}

// ---------------------------------------------------------------------------
// Applicability
// ---------------------------------------------------------------------------

/// Why one release does or does not apply to one install.
///
/// A single enum rather than a `bool` because every one of these answers ends up
/// in a support conversation: "why did my Mac not get 1.4.0" has six different
/// answers, and an updater that cannot tell them apart cannot answer any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applicability {
    /// It applies. Offer it.
    Applies,
    /// Not newer than what is already installed — the common case, and the one
    /// that fires for every release below the current version on every check.
    NotNewer,
    /// It carries a pre-release tag and this install did not opt in.
    PreRelease,
    /// The release needs a newer operating system than this machine runs.
    OsTooOld {
        /// The floor the release declared.
        needs: Version,
        /// What this machine reports.
        running: Version,
    },
    /// Nothing in the release is built for this platform.
    NoArtifact {
        /// The platform that found nothing.
        platform: Platform,
    },
    /// The user asked not to be shown this version again.
    SkippedByUser,
    /// The staged rollout has not reached this install yet.
    OutsideRollout {
        /// This install's bucket.
        bucket: u8,
        /// How far the rollout has been opened.
        rollout: u8,
    },
}

impl Applicability {
    /// Whether this is [`Applicability::Applies`].
    pub fn applies(&self) -> bool {
        matches!(self, Applicability::Applies)
    }
}

impl fmt::Display for Applicability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Applicability::Applies => f.write_str("applies"),
            Applicability::NotNewer => f.write_str("not newer than the running version"),
            Applicability::PreRelease => {
                f.write_str("is a pre-release and this install did not opt in")
            }
            Applicability::OsTooOld { needs, running } => {
                write!(f, "needs OS {needs}, this machine runs {running}")
            }
            Applicability::NoArtifact { platform } => {
                write!(f, "has no artifact for {platform}")
            }
            Applicability::SkippedByUser => f.write_str("was skipped by the user"),
            Applicability::OutsideRollout { bucket, rollout } => {
                write!(
                    f,
                    "rollout is at {rollout}%, this install is in bucket {bucket}"
                )
            }
        }
    }
}

/// Whether one release applies to one install, and why not when it does not.
///
/// The order of the checks is the order a human would explain them in, and it is
/// load-bearing for the message: a release that is both older *and* built for
/// another platform reports "not newer", because that is the true reason nobody
/// will ever install it.
///
/// **`mandatory` bypasses exactly two of these**: the user's skip list and the
/// staged rollout. It does not bypass the channel, the OS floor or the missing
/// artifact, because those three describe installs that would fail rather than
/// installs that said no.
pub fn applicability(release: &Release, install: &Install) -> Applicability {
    if release.version() <= install.version() {
        return Applicability::NotNewer;
    }
    if release.version().is_pre_release() && !install.pre_release {
        return Applicability::PreRelease;
    }
    let os = install.platform.os();
    if !release.minimum_os().allows(os, install.os_version.as_ref()) {
        // `allows` already returned false, so both sides are present.
        let needs = release
            .minimum_os()
            .for_os(os)
            .cloned()
            .unwrap_or(Version::ZERO);
        let running = install.os_version.clone().unwrap_or(Version::ZERO);
        return Applicability::OsTooOld { needs, running };
    }
    if release.artifact_for(install.platform.clone()).is_none() {
        return Applicability::NoArtifact {
            platform: install.platform.clone(),
        };
    }
    if !release.is_mandatory() {
        if install.skipped.contains(release.version()) {
            return Applicability::SkippedByUser;
        }
        if install.bucket >= release.rollout() {
            return Applicability::OutsideRollout {
                bucket: install.bucket,
                rollout: release.rollout(),
            };
        }
    }
    Applicability::Applies
}

// ---------------------------------------------------------------------------
// choose
// ---------------------------------------------------------------------------

/// The newest release that applies to this install, if any.
///
/// Walks the feed newest-first ([`Feed::parse`] already sorted it) and stops at
/// the first release [`applicability`] accepts.
///
/// It does step *over* a release that does not apply and offer an older one:
/// an install on 1.4.0 should get the 1.4.1 security fix while 1.5.0 is still at
/// 5% rollout, and refusing to would hold it on a build with a known bug. What
/// can never happen is moving *below* what is already installed —
/// [`Applicability::NotNewer`] rejects that before any other check runs, so the
/// worst case is being offered nothing.
pub fn choose(feed: &Feed, install: &Install) -> Result<Option<Offer>, ChooseError> {
    if feed.app() != install.app {
        return Err(ChooseError::WrongApp {
            expected: install.app.clone(),
            found: feed.app().to_string(),
        });
    }
    if feed.channel() != install.channel {
        return Err(ChooseError::WrongChannel {
            expected: install.channel.clone(),
            found: feed.channel().to_string(),
        });
    }

    for release in feed.releases() {
        if !applicability(release, install).applies() {
            continue;
        }
        let artifact = match release.artifact_for(install.platform.clone()) {
            Some(artifact) => artifact.clone(),
            // Unreachable: `applicability` already rejected a release with no
            // artifact. Written as a `continue` rather than an `unwrap` because
            // an updater that panics on a surprising feed is an application that
            // will not start.
            None => continue,
        };
        let delta = if install.deltas {
            artifact.delta_from(&install.version).cloned()
        } else {
            None
        };
        return Ok(Some(Offer {
            release: release.clone(),
            artifact,
            delta,
        }));
    }
    Ok(None)
}

/// Every release in the feed with the reason it was or was not chosen.
///
/// The diagnostic sibling of [`choose`] — what a `--why-no-update` flag prints,
/// and what turns a support ticket into one line of output.
pub fn explain(feed: &Feed, install: &Install) -> Vec<(Version, Applicability)> {
    feed.releases()
        .iter()
        .map(|release| (release.version().clone(), applicability(release, install)))
        .collect()
}

/// Why a feed could not be used for this install at all.
///
/// Both variants describe a *misconfiguration*, not a missing update, which is
/// why they are an error rather than "no update available": a server handing out
/// the wrong feed should be loud on the first check, not quietly silent forever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChooseError {
    /// The feed belongs to a different application.
    WrongApp {
        /// The identifier this install expected.
        expected: String,
        /// The identifier the feed carried.
        found: String,
    },
    /// The feed serves a different channel.
    WrongChannel {
        /// The channel this install follows.
        expected: String,
        /// The channel the feed serves.
        found: String,
    },
}

impl fmt::Display for ChooseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChooseError::WrongApp { expected, found } => {
                write!(f, "feed is for `{found}`, this install is `{expected}`")
            }
            ChooseError::WrongChannel { expected, found } => write!(
                f,
                "feed serves the `{found}` channel, this install follows `{expected}`"
            ),
        }
    }
}

impl std::error::Error for ChooseError {}

// ---------------------------------------------------------------------------
// Offer
// ---------------------------------------------------------------------------

/// One release, one file, ready to download.
///
/// It owns its data rather than borrowing the feed: the feed is a response body
/// that will be dropped, and an offer outlives it — it is shown in a dialog,
/// carried across a download, and written to [`crate::pending`].
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    release: Release,
    artifact: Artifact,
    delta: Option<Delta>,
}

impl Offer {
    /// The version being offered.
    pub fn version(&self) -> &Version {
        self.release.version()
    }

    /// The whole release, for the notes, the date and the mandatory flag.
    pub fn release(&self) -> &Release {
        &self.release
    }

    /// The full artifact — the file to install when there is no delta, and the
    /// thing a delta patches *into* when there is.
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }

    /// The delta chosen for this install, if deltas are enabled and one matched.
    pub fn delta(&self) -> Option<&Delta> {
        self.delta.as_ref()
    }

    /// Whether this offer downloads a patch rather than the whole file.
    pub fn is_delta(&self) -> bool {
        self.delta.is_some()
    }

    /// Whether the publisher marked the release as one users should not skip.
    pub fn is_mandatory(&self) -> bool {
        self.release.is_mandatory()
    }

    /// What to fetch — the delta's URL when there is one, the artifact's
    /// otherwise. Every accessor below follows the same rule, so a downloader
    /// never has to ask which of the two it is holding.
    pub fn url(&self) -> &str {
        match &self.delta {
            Some(delta) => delta.url(),
            None => self.artifact.url(),
        }
    }

    /// How many bytes to expect.
    pub fn size(&self) -> u64 {
        match &self.delta {
            Some(delta) => delta.size(),
            None => self.artifact.size(),
        }
    }

    /// The digest the downloaded bytes must have.
    pub fn sha256(&self) -> Digest {
        match &self.delta {
            Some(delta) => delta.sha256(),
            None => self.artifact.sha256(),
        }
    }

    /// The detached signature the feed published, base64 as written.
    pub fn signature(&self) -> Option<&str> {
        match &self.delta {
            Some(delta) => delta.signature(),
            None => self.artifact.signature(),
        }
    }

    /// The signature decoded to bytes, or `None` when it is absent or malformed.
    ///
    /// Absent and malformed are the same answer here on purpose: both are
    /// "there is no signature to check", and [`Download::finish`] turns both into
    /// a refusal when a verifier exists.
    pub fn signature_bytes(&self) -> Option<Vec<u8>> {
        decode_base64(self.signature()?)
    }

    /// A fresh [`Download`] that will check exactly this offer.
    pub fn download(&self) -> Download {
        Download {
            expected_size: self.size(),
            expected_digest: self.sha256(),
            signature: self.signature().map(str::to_string),
            hasher: Sha256::new(),
            received: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// A download being checked as it arrives.
///
/// The point of hashing incrementally is that a 200 MB installer never has to be
/// resident in memory to be verified. The point of checking the size *first* is
/// that a truncated or oversized download is rejected without hashing it at all.
///
/// ```
/// use silka_dist::sha256::sha256;
/// use silka_dist::update::Download;
///
/// let payload = b"an installer, pretend";
/// let mut download = Download::expecting(payload.len() as u64, sha256(payload));
/// download.write(&payload[..5]).unwrap();
/// download.write(&payload[5..]).unwrap();
/// download.finish(None).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct Download {
    expected_size: u64,
    expected_digest: Digest,
    signature: Option<String>,
    hasher: Sha256,
    received: u64,
}

impl Download {
    /// A download that expects a size and a digest, with no signature.
    ///
    /// The shape tests and one-off fetches want. A real update download comes
    /// from [`Offer::download`], which carries the signature too.
    pub fn expecting(size: u64, digest: Digest) -> Download {
        Download {
            expected_size: size,
            expected_digest: digest,
            signature: None,
            hasher: Sha256::new(),
            received: 0,
        }
    }

    /// Attach the detached signature the feed published (base64).
    pub fn signed(mut self, signature: impl Into<String>) -> Download {
        self.signature = Some(signature.into());
        self
    }

    /// How many bytes have arrived so far — the numerator of a progress bar.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// How many bytes are expected in total.
    pub fn expected(&self) -> u64 {
        self.expected_size
    }

    /// Feed a chunk.
    ///
    /// Fails the moment more bytes have arrived than were promised, rather than
    /// at the end: a server that keeps sending is a server that can fill a disk,
    /// and the honest place to stop is the first byte past the limit.
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), VerifyError> {
        let after = self.received.saturating_add(chunk.len() as u64);
        if after > self.expected_size {
            return Err(VerifyError::TooLong {
                expected: self.expected_size,
                received: after,
            });
        }
        self.hasher.update(chunk);
        self.received = after;
        Ok(())
    }

    /// Check everything and hand back the digest that was computed.
    ///
    /// The order is size, then digest, then signature — cheapest first, and each
    /// one is a strict precondition of the next. `verifier` is `Option` because
    /// a build may genuinely have none; when it is `Some`, a missing or
    /// malformed signature is a **refusal**, never a pass.
    pub fn finish(self, verifier: Option<&dyn SignatureVerifier>) -> Result<Digest, VerifyError> {
        if self.received != self.expected_size {
            return Err(VerifyError::SizeMismatch {
                expected: self.expected_size,
                actual: self.received,
            });
        }
        let actual = self.hasher.finish();
        if actual != self.expected_digest {
            return Err(VerifyError::DigestMismatch {
                expected: self.expected_digest,
                actual,
            });
        }
        if let Some(verifier) = verifier {
            let text = match &self.signature {
                Some(text) => text,
                None => return Err(VerifyError::MissingSignature),
            };
            let bytes = match decode_base64(text) {
                Some(bytes) => bytes,
                None => return Err(VerifyError::MalformedSignature),
            };
            // The signed message is the digest, not the file: it is 32 bytes,
            // it is what the release pipeline signed, and signing it means a
            // verifier never has to hold the whole artifact in memory either.
            if !verifier.verify(actual.as_bytes(), &bytes) {
                return Err(VerifyError::BadSignature);
            }
        }
        Ok(actual)
    }
}

/// Verify a complete buffer in one call.
///
/// The convenience form of [`Download`] for a fetch that already produced a
/// `Vec<u8>`. Same checks, same order.
pub fn verify(
    offer: &Offer,
    bytes: &[u8],
    verifier: Option<&dyn SignatureVerifier>,
) -> Result<Digest, VerifyError> {
    let mut download = offer.download();
    download.write(bytes)?;
    download.finish(verifier)
}

/// The application's signature check.
///
/// Implemented with a real cryptography crate — `ed25519-dalek`, `ring`, a
/// platform API. The `message` is the artifact's 32-byte SHA-256 digest and the
/// `signature` is the feed's field already base64-decoded, so an implementation
/// is one call into a library and no parsing of its own.
///
/// ```
/// use silka_dist::update::SignatureVerifier;
///
/// /// Stands in for a real one in tests: accepts a signature that is the
/// /// digest reversed. Never write anything like this in a shipped build.
/// struct Reversed;
/// impl SignatureVerifier for Reversed {
///     fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
///         signature.iter().rev().eq(message.iter())
///     }
/// }
/// ```
pub trait SignatureVerifier {
    /// Whether `signature` is a valid signature over `message` by a key this
    /// verifier trusts. Returns `bool` rather than `Result` because there is
    /// exactly one useful answer to "the signature did not check out".
    fn verify(&self, message: &[u8], signature: &[u8]) -> bool;
}

/// Why a downloaded file was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// More bytes arrived than the feed promised; the download was stopped.
    TooLong {
        /// The promised length.
        expected: u64,
        /// How much had arrived when it was stopped.
        received: u64,
    },
    /// The download ended short of the promised length.
    SizeMismatch {
        /// The promised length.
        expected: u64,
        /// What actually arrived.
        actual: u64,
    },
    /// The bytes hashed to something else — a corrupt mirror, a stale cache, a
    /// proxy that served the previous release.
    DigestMismatch {
        /// The digest the feed published.
        expected: Digest,
        /// The digest of what arrived.
        actual: Digest,
    },
    /// A verifier was supplied but the feed published no signature.
    MissingSignature,
    /// The signature field was not valid base64.
    MalformedSignature,
    /// The signature did not verify. This is the one that means *attack* rather
    /// than *accident*, and the only correct reaction is to delete the file.
    BadSignature,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::TooLong { expected, received } => write!(
                f,
                "download is longer than the {expected} bytes the feed promised ({received} and still arriving)"
            ),
            VerifyError::SizeMismatch { expected, actual } => {
                write!(f, "download is {actual} bytes, the feed promised {expected}")
            }
            VerifyError::DigestMismatch { expected, actual } => {
                write!(f, "download hashes to {actual}, the feed promised {expected}")
            }
            VerifyError::MissingSignature => {
                f.write_str("the feed published no signature for this artifact")
            }
            VerifyError::MalformedSignature => {
                f.write_str("the signature field is not valid base64")
            }
            VerifyError::BadSignature => f.write_str("the signature did not verify"),
        }
    }
}

impl std::error::Error for VerifyError {}

// ---------------------------------------------------------------------------
// base64
// ---------------------------------------------------------------------------

/// Decode standard base64 (`A–Z a–z 0–9 + /`, `=` padding).
///
/// Strict: it rejects the URL-safe alphabet, characters after the padding, and
/// trailing bits that are not zero. Whitespace is skipped, because a signature
/// that travelled through a YAML file arrives wrapped.
///
/// Public because a caller that stores the signature itself needs the same
/// decoder, and because a decoder nobody can test is a decoder nobody trusts.
///
/// ```
/// use silka_dist::update::decode_base64;
///
/// assert_eq!(decode_base64("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
/// assert_eq!(decode_base64("aGVsbG8_"), None, "URL-safe alphabet is not this one");
/// ```
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut padding = 0usize;
    let mut symbols = 0usize;

    for byte in text.bytes() {
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b'=' => {
                padding += 1;
                continue;
            }
            _ => {}
        }
        if padding > 0 {
            // Data after `=` means two messages were concatenated.
            return None;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        symbols += 1;
        buffer = ((buffer << 6) | u32::from(value)) & 0xffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }

    if padding > 2 {
        return None;
    }
    // A single leftover symbol encodes six bits, which is not a byte and not
    // nothing: it is a truncated message.
    if bits >= 6 {
        return None;
    }
    if bits > 0 && (buffer & ((1u32 << bits) - 1)) != 0 {
        return None;
    }
    // Padding must land the message on a four-symbol boundary.
    if padding > 0 && (symbols + padding) % 4 != 0 {
        return None;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Os;

    const DIGEST_A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const DIGEST_B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn feed_document(releases: &str) -> String {
        format!(
            r#"{{"feed": 1, "app": "dev.silka.dashboard", "channel": "stable",
                "releases": [{releases}]}}"#
        )
    }

    fn mac_release(version: &str, extra: &str) -> String {
        format!(
            r#"{{"version": "{version}"{extra},
                "artifacts": [{{"platform": "macos-universal", "format": "dmg",
                    "url": "https://example.com/{version}.dmg", "size": 100,
                    "sha256": "{DIGEST_A}", "signature": "aGVsbG8=",
                    "deltas": [{{"from": "1.3.0",
                        "url": "https://example.com/{version}.delta", "size": 20,
                        "sha256": "{DIGEST_B}"}}]}}]}}"#
        )
    }

    fn feed_of(releases: &str) -> Feed {
        Feed::parse(&feed_document(releases)).expect("feed contoh harus terbaca")
    }

    fn install_at(version: &str) -> Install {
        Install::new(
            "dev.silka.dashboard",
            Version::parse(version).expect("versi contoh"),
        )
        .platform(Platform::MacosArm64)
    }

    // -- choose ------------------------------------------------------------

    #[test]
    fn menawarkan_rilis_terbaru_yang_berlaku() {
        let feed = feed_of(&format!(
            "{},{}",
            mac_release("1.4.0", ""),
            mac_release("1.5.0", "")
        ));
        let offer = choose(&feed, &install_at("1.3.0")).unwrap().unwrap();
        assert_eq!(offer.version(), &Version::new(1, 5, 0));
        assert_eq!(offer.url(), "https://example.com/1.5.0.dmg");
        assert_eq!(offer.size(), 100);
        assert!(!offer.is_delta());
    }

    #[test]
    fn tidak_ada_yang_lebih_baru() {
        let feed = feed_of(&mac_release("1.4.0", ""));
        assert_eq!(choose(&feed, &install_at("1.4.0")).unwrap(), None);
        assert_eq!(choose(&feed, &install_at("2.0.0")).unwrap(), None);
    }

    #[test]
    fn feed_aplikasi_lain_ditolak_bukan_diam() {
        let feed = feed_of(&mac_release("1.4.0", ""));
        let install = Install::new("dev.silka.lain", Version::new(1, 0, 0));
        assert!(matches!(
            choose(&feed, &install),
            Err(ChooseError::WrongApp { .. })
        ));
    }

    #[test]
    fn kanal_yang_tidak_cocok_ditolak() {
        let feed = feed_of(&mac_release("1.4.0", ""));
        let install = install_at("1.3.0").channel("beta");
        let error = choose(&feed, &install).unwrap_err();
        assert!(matches!(error, ChooseError::WrongChannel { .. }));
        assert!(error.to_string().contains("beta"));
    }

    #[test]
    fn pra_rilis_hanya_untuk_yang_ikut_serta() {
        let feed = feed_of(&mac_release("1.5.0-rc.1", ""));
        assert_eq!(choose(&feed, &install_at("1.4.0")).unwrap(), None);

        let install = install_at("1.4.0").pre_release(true);
        assert!(choose(&feed, &install).unwrap().is_some());
    }

    #[test]
    fn lantai_os_menahan_rilis() {
        let feed = feed_of(&mac_release(
            "1.5.0",
            r#", "minimum_os": {"macos": "13.0"}"#,
        ));

        let too_old = install_at("1.4.0").os_version(Version::parse("12.6").unwrap());
        assert_eq!(choose(&feed, &too_old).unwrap(), None);

        let new_enough = install_at("1.4.0").os_version(Version::parse("13.1").unwrap());
        assert!(choose(&feed, &new_enough).unwrap().is_some());

        // Unknown OS version is allowed through: stranding is the worse failure.
        assert!(choose(&feed, &install_at("1.4.0")).unwrap().is_some());
    }

    #[test]
    fn platform_tanpa_artefak_tidak_ditawari() {
        let feed = feed_of(&mac_release("1.5.0", ""));
        let windows = install_at("1.4.0").platform(Platform::WindowsX64);
        assert_eq!(choose(&feed, &windows).unwrap(), None);
    }

    #[test]
    fn rollout_bertahap_menyaring_lewat_bucket() {
        let feed = feed_of(&mac_release("1.5.0", r#", "rollout": 25"#));

        let inside = install_at("1.4.0").bucket(10);
        assert!(choose(&feed, &inside).unwrap().is_some());

        let outside = install_at("1.4.0").bucket(25);
        assert_eq!(choose(&feed, &outside).unwrap(), None);

        // Rollout 0 reaches nobody, 100 reaches every bucket 0..=99.
        let none = feed_of(&mac_release("1.5.0", r#", "rollout": 0"#));
        assert_eq!(choose(&none, &install_at("1.4.0")).unwrap(), None);
        let all = feed_of(&mac_release("1.5.0", r#", "rollout": 100"#));
        assert!(choose(&all, &install_at("1.4.0").bucket(99))
            .unwrap()
            .is_some());
    }

    #[test]
    fn versi_yang_dilewati_tidak_ditawarkan_lagi() {
        let feed = feed_of(&mac_release("1.5.0", ""));
        let install = install_at("1.4.0").skip(Version::new(1, 5, 0));
        assert_eq!(choose(&feed, &install).unwrap(), None);
    }

    #[test]
    fn wajib_menembus_lewatan_dan_rollout_tapi_bukan_lantai_os() {
        let mandatory = r#", "mandatory": true, "rollout": 1"#;
        let feed = feed_of(&mac_release("1.5.0", mandatory));

        let install = install_at("1.4.0").skip(Version::new(1, 5, 0)).bucket(99);
        let offer = choose(&feed, &install).unwrap().expect("wajib harus lewat");
        assert!(offer.is_mandatory());

        let blocked = feed_of(&mac_release(
            "1.5.0",
            r#", "mandatory": true, "minimum_os": {"macos": "13.0"}"#,
        ));
        let old_mac = install_at("1.4.0").os_version(Version::parse("12.0").unwrap());
        assert_eq!(
            choose(&blocked, &old_mac).unwrap(),
            None,
            "wajib tidak boleh menembus lantai OS: instalasinya akan gagal jalan"
        );
    }

    #[test]
    fn rilis_lebih_lama_yang_penuh_ditawarkan_saat_yang_terbaru_masih_tertahan() {
        // 1.5.0 is held back by its rollout; 1.4.1 is wide open. Offering 1.4.1
        // is the point: it may be the security fix, and waiting for 1.5.0 to
        // open would hold this install on a build with a known bug.
        let feed = feed_of(&format!(
            "{},{}",
            mac_release("1.4.1", ""),
            mac_release("1.5.0", r#", "rollout": 5"#)
        ));
        let install = install_at("1.4.0").bucket(50);
        let offer = choose(&feed, &install).unwrap();
        assert_eq!(
            offer.map(|chosen| chosen.version().clone()),
            Some(Version::parse("1.4.1").unwrap())
        );
    }

    #[test]
    fn tidak_pernah_mundur_di_bawah_versi_yang_terpasang() {
        // The rule that does hold absolutely: nothing below the running version
        // is ever offered, whatever its rollout says.
        let feed = feed_of(&format!(
            "{},{}",
            mac_release("1.4.1", ""),
            mac_release("1.5.0", r#", "rollout": 5"#)
        ));
        let install = install_at("1.4.1").bucket(50);
        assert_eq!(choose(&feed, &install).unwrap(), None);
    }

    // -- deltas ------------------------------------------------------------

    #[test]
    fn delta_mati_secara_bawaan() {
        let feed = feed_of(&mac_release("1.5.0", ""));
        let offer = choose(&feed, &install_at("1.3.0")).unwrap().unwrap();
        assert!(!offer.is_delta());
        assert_eq!(offer.url(), "https://example.com/1.5.0.dmg");
    }

    #[test]
    fn delta_dipakai_saat_dinyalakan_dan_cocok() {
        let feed = feed_of(&mac_release("1.5.0", ""));

        let matching = install_at("1.3.0").deltas(true);
        let offer = choose(&feed, &matching).unwrap().unwrap();
        assert!(offer.is_delta());
        assert_eq!(offer.url(), "https://example.com/1.5.0.delta");
        assert_eq!(offer.size(), 20);
        assert_eq!(offer.sha256(), Digest::parse(DIGEST_B).unwrap());

        // A version with no delta published falls back to the full file — that
        // is not an error, it is a bigger download.
        let other = install_at("1.2.0").deltas(true);
        let offer = choose(&feed, &other).unwrap().unwrap();
        assert!(!offer.is_delta());
        assert_eq!(offer.size(), 100);
    }

    // -- applicability / explain -------------------------------------------

    #[test]
    fn applicability_menyebut_alasannya() {
        let feed = feed_of(&mac_release("1.5.0", r#", "rollout": 5"#));
        let release = &feed.releases()[0];

        assert_eq!(
            applicability(release, &install_at("1.6.0")),
            Applicability::NotNewer
        );
        assert_eq!(
            applicability(release, &install_at("1.4.0").bucket(50)),
            Applicability::OutsideRollout {
                bucket: 50,
                rollout: 5
            }
        );
        assert_eq!(
            applicability(
                release,
                &install_at("1.4.0").platform(Platform::LinuxX64).bucket(0)
            ),
            Applicability::NoArtifact {
                platform: Platform::LinuxX64
            }
        );
        assert!(applicability(release, &install_at("1.4.0")).applies());
    }

    #[test]
    fn applicability_os_terlalu_tua_membawa_dua_angkanya() {
        let feed = feed_of(&mac_release(
            "1.5.0",
            r#", "minimum_os": {"macos": "13.0"}"#,
        ));
        let install = install_at("1.4.0").os_version(Version::parse("12.6").unwrap());
        match applicability(&feed.releases()[0], &install) {
            Applicability::OsTooOld { needs, running } => {
                assert_eq!(needs, Version::new(13, 0, 0));
                assert_eq!(running, Version::parse("12.6").unwrap());
            }
            other => panic!("harus OsTooOld, dapat {other:?}"),
        }
    }

    #[test]
    fn explain_meliput_setiap_rilis() {
        let feed = feed_of(&format!(
            "{},{}",
            mac_release("1.4.0", ""),
            mac_release("1.5.0", "")
        ));
        let rows = explain(&feed, &install_at("1.4.0"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, Version::new(1, 5, 0));
        assert!(rows[0].1.applies());
        assert_eq!(rows[1].1, Applicability::NotNewer);
        assert!(rows[1].1.to_string().contains("newer"));
    }

    #[test]
    fn os_diambil_dari_platform_bukan_dari_pemanggil() {
        // A Windows floor must never be compared against a Mac's version.
        let feed = feed_of(&mac_release(
            "1.5.0",
            r#", "minimum_os": {"windows": "10.0.99999"}"#,
        ));
        let mac = install_at("1.4.0").os_version(Version::parse("12.0").unwrap());
        assert!(choose(&feed, &mac).unwrap().is_some());
        assert_eq!(Platform::MacosArm64.os(), Os::Macos);
    }

    // -- buckets -----------------------------------------------------------

    #[test]
    fn bucket_stabil_dan_dalam_jangkauan() {
        for id in ["a", "install-uuid", "", "🙂"] {
            let bucket = bucket_for(id);
            assert!(bucket < 100);
            assert_eq!(bucket, bucket_for(id), "bucket harus stabil");
        }
    }

    #[test]
    fn bucket_tersebar_merata() {
        // 1000 identifiers should not pile into a corner. A uniform draw puts
        // ~100 in each decile; the bound here is loose enough to never flake and
        // tight enough to catch a mapping that collapses.
        let mut deciles = [0usize; 10];
        for index in 0..1000 {
            deciles[(bucket_for(&format!("install-{index}")) / 10) as usize] += 1;
        }
        for count in deciles {
            assert!(
                (40..=180).contains(&count),
                "sebaran bucket timpang: {deciles:?}"
            );
        }
    }

    #[test]
    fn identifier_mengisi_bucket() {
        let install = install_at("1.0.0").identifier("install-uuid");
        assert_eq!(install.bucket_index(), bucket_for("install-uuid"));
    }

    #[test]
    fn bucket_di_atas_99_dijepit() {
        assert_eq!(install_at("1.0.0").bucket(200).bucket_index(), 99);
    }

    #[test]
    fn menaikkan_rollout_tidak_pernah_mencabut_tawaran() {
        // The property that makes a hash bucket the right tool: everyone offered
        // at 10% is still offered at 25%.
        let ten = feed_of(&mac_release("1.5.0", r#", "rollout": 10"#));
        let twenty_five = feed_of(&mac_release("1.5.0", r#", "rollout": 25"#));
        for index in 0..200 {
            let install = install_at("1.4.0").identifier(&format!("install-{index}"));
            if choose(&ten, &install).unwrap().is_some() {
                assert!(
                    choose(&twenty_five, &install).unwrap().is_some(),
                    "install-{index} kehilangan tawaran saat rollout dinaikkan"
                );
            }
        }
    }

    // -- accessors ---------------------------------------------------------

    #[test]
    fn install_mengembalikan_apa_yang_disetel() {
        let install = Install::new("app", Version::new(1, 0, 0))
            .channel("beta")
            .platform(Platform::LinuxX64)
            .os_version(Version::new(6, 1, 0))
            .pre_release(true)
            .deltas(true)
            .skip(Version::new(1, 1, 0))
            .skip(Version::new(1, 1, 0));

        assert_eq!(install.app_id(), "app");
        assert_eq!(install.version(), &Version::new(1, 0, 0));
        assert_eq!(install.channel_name(), "beta");
        assert_eq!(install.platform_name(), &Platform::LinuxX64);
        assert_eq!(install.os(), Some(&Version::new(6, 1, 0)));
        assert!(install.takes_pre_release());
        assert!(install.takes_deltas());
        assert_eq!(install.skipped().len(), 1, "lewatan ganda tidak menumpuk");
    }

    // -- download / verify -------------------------------------------------

    struct Reversed;
    impl SignatureVerifier for Reversed {
        fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
            signature.iter().rev().eq(message.iter())
        }
    }

    struct NeverTrusts;
    impl SignatureVerifier for NeverTrusts {
        fn verify(&self, _message: &[u8], _signature: &[u8]) -> bool {
            false
        }
    }

    /// The two verifiers as trait objects.
    ///
    /// Spelled out rather than written inline at each call site: `Some(&Reversed)`
    /// relies on the expected type propagating an unsized coercion through the
    /// `Some`, and a test whose subject is signature checking should not also be
    /// a test of type inference.
    fn reversed() -> &'static dyn SignatureVerifier {
        &Reversed
    }

    fn never_trusts() -> &'static dyn SignatureVerifier {
        &NeverTrusts
    }

    #[test]
    fn unduhan_yang_benar_lolos() {
        let payload = b"halo dunia yang cukup panjang untuk dua potongan";
        let mut download = Download::expecting(payload.len() as u64, sha256(payload));
        download.write(&payload[..10]).unwrap();
        assert_eq!(download.received(), 10);
        assert_eq!(download.expected(), payload.len() as u64);
        download.write(&payload[10..]).unwrap();
        assert_eq!(download.finish(None).unwrap(), sha256(payload));
    }

    #[test]
    fn unduhan_pendek_ditolak() {
        let payload = b"halo dunia";
        let mut download = Download::expecting(payload.len() as u64, sha256(payload));
        download.write(&payload[..4]).unwrap();
        assert_eq!(
            download.finish(None),
            Err(VerifyError::SizeMismatch {
                expected: 10,
                actual: 4
            })
        );
    }

    #[test]
    fn unduhan_kepanjangan_dihentikan_di_byte_pertama_yang_kelebihan() {
        let mut download = Download::expecting(4, sha256(b"abcd"));
        let error = download.write(b"abcdefgh").unwrap_err();
        assert!(matches!(error, VerifyError::TooLong { expected: 4, .. }));
        assert_eq!(
            download.received(),
            0,
            "potongan yang ditolak tidak dihitung"
        );
    }

    #[test]
    fn digest_yang_tidak_cocok_ditolak() {
        let mut download = Download::expecting(4, sha256(b"abcd"));
        download.write(b"abce").unwrap();
        assert!(matches!(
            download.finish(None),
            Err(VerifyError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn tanda_tangan_diperiksa_saat_ada_verifier() {
        let payload = b"muatan";
        let digest = sha256(payload);
        let reversed_digest: Vec<u8> = digest.as_bytes().iter().rev().copied().collect();
        let signature = encode_base64_for_test(&reversed_digest);

        let mut download =
            Download::expecting(payload.len() as u64, digest).signed(signature.clone());
        download.write(payload).unwrap();
        assert!(download.finish(Some(reversed())).is_ok());

        let mut download = Download::expecting(payload.len() as u64, digest).signed(signature);
        download.write(payload).unwrap();
        assert_eq!(
            download.finish(Some(never_trusts())),
            Err(VerifyError::BadSignature)
        );
    }

    #[test]
    fn tanda_tangan_yang_hilang_adalah_penolakan_bukan_kelolosan() {
        let payload = b"muatan";
        let mut download = Download::expecting(payload.len() as u64, sha256(payload));
        download.write(payload).unwrap();
        assert_eq!(
            download.finish(Some(reversed())),
            Err(VerifyError::MissingSignature),
            "artefak tanpa tanda tangan tidak boleh lolos hanya karena bidangnya kosong"
        );
    }

    #[test]
    fn tanda_tangan_rusak_ditolak() {
        let payload = b"muatan";
        let mut download =
            Download::expecting(payload.len() as u64, sha256(payload)).signed("bukan base64!");
        download.write(payload).unwrap();
        assert_eq!(
            download.finish(Some(reversed())),
            Err(VerifyError::MalformedSignature)
        );
    }

    #[test]
    fn tanpa_verifier_hanya_integritas_yang_dijamin() {
        let payload = b"muatan";
        let mut download = Download::expecting(payload.len() as u64, sha256(payload));
        download.write(payload).unwrap();
        assert!(download.finish(None).is_ok());
    }

    #[test]
    fn verify_sekali_jalan_sama_dengan_streaming() {
        let feed = feed_of(&mac_release("1.5.0", ""));
        let offer = choose(&feed, &install_at("1.4.0")).unwrap().unwrap();
        // The sample feed says 100 bytes with the digest of "abc"; only the
        // shape of the failure matters here.
        assert!(matches!(
            verify(&offer, b"abc", None),
            Err(VerifyError::SizeMismatch {
                expected: 100,
                actual: 3
            })
        ));
    }

    #[test]
    fn galat_verifikasi_punya_pesan() {
        assert!(VerifyError::BadSignature.to_string().contains("verify"));
        assert!(VerifyError::MissingSignature
            .to_string()
            .contains("signature"));
    }

    #[test]
    fn offer_meneruskan_bidang_delta_secara_utuh() {
        let feed = feed_of(&mac_release("1.5.0", ""));
        let offer = choose(&feed, &install_at("1.4.0")).unwrap().unwrap();
        assert_eq!(offer.artifact().format(), "dmg");
        assert_eq!(offer.signature(), Some("aGVsbG8="));
        assert_eq!(offer.signature_bytes().as_deref(), Some(&b"hello"[..]));
        assert!(offer.delta().is_none());
        assert_eq!(offer.release().version(), &Version::new(1, 5, 0));
    }

    // -- base64 ------------------------------------------------------------

    /// Encoder used only to build test fixtures; the crate itself never encodes.
    fn encode_base64_for_test(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
            out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[(triple >> 6) as usize & 63] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[triple as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    #[test]
    fn base64_vektor_rfc4648() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(
                decode_base64(encoded).as_deref(),
                Some(plain.as_bytes()),
                "vektor {encoded:?}"
            );
        }
    }

    #[test]
    fn base64_bolak_balik_untuk_setiap_panjang() {
        for length in 0..40usize {
            let bytes: Vec<u8> = (0..length).map(|index| (index * 7 + 3) as u8).collect();
            let encoded = encode_base64_for_test(&bytes);
            assert_eq!(decode_base64(&encoded), Some(bytes), "panjang {length}");
        }
    }

    #[test]
    fn base64_melewati_spasi_yang_dibawa_yaml() {
        assert_eq!(
            decode_base64("Zm9v\n  YmFy\n").as_deref(),
            Some(&b"foobar"[..])
        );
    }

    #[test]
    fn base64_yang_ditolak() {
        for text in [
            "Zg=",       // padding that does not land on a four-symbol boundary
            "Zg===",     // three pads is never valid
            "Zm9v=YmFy", // data after the padding
            "aGVsbG8_",  // URL-safe alphabet is a different encoding
            "Zm9v YmF*", // a character outside the alphabet
            "Z",         // a lone symbol is six bits: neither a byte nor nothing
        ] {
            assert!(
                decode_base64(text).is_none(),
                "{text:?} seharusnya ditolak, bukan diterima"
            );
        }
    }

    #[test]
    fn base64_tanpa_padding_tetap_diterima() {
        // Some signing tools emit unpadded base64. Accepting it costs nothing;
        // the leftover-bit check below is what actually keeps it honest.
        assert_eq!(decode_base64("Zm9vYg").as_deref(), Some(&b"foob"[..]));
        assert_eq!(decode_base64("Zm9vYmE").as_deref(), Some(&b"fooba"[..]));
    }

    #[test]
    fn base64_bit_sisa_yang_tidak_nol_ditolak() {
        // "Zh==" would decode to one byte plus four non-zero bits.
        assert_eq!(decode_base64("Zh=="), None);
    }
}
