//! # silka-dist
//!
//! The half of shipping software that runs **inside** the shipped binary.
//!
//! Distribution (SISA-PEKERJAAN §I, INTEGRASI-NATIVE §9) is mostly not code:
//! signing, notarizing, bundling and uploading are
//! `.github/workflows/release.yml`, the scripts beside it, and
//! `docs/RELEASE.md`, which walks a release from zero. But four of those steps
//! have a counterpart the application itself performs, and a counterpart that
//! runs on a user's machine is a counterpart that has to be *tested*:
//!
//! | Module | The question it answers |
//! |---|---|
//! | [`version`] | Is `1.4.0-rc.2` newer than `1.4.0-rc.10`? (no, and that is the point) |
//! | [`feed`] | What did the release pipeline publish? |
//! | [`update`] | Which of those releases applies to *this* install, on *this* OS, in *this* rollout bucket — and are these the bytes it named? |
//! | [`sha256`] | Is the file we downloaded byte-for-byte the file the feed described? |
//! | [`pending`] | What has to happen at the next restart, and what if the swap fails? |
//! | [`crash`] | What is written down before the process dies, and where is it read back? |
//! | [`json`] | The one document format all of the above are written in |
//!
//! ## The shape of a check, end to end
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
//!     "rollout": 25,
//!     "minimum_os": { "macos": "12.0" },
//!     "artifacts": [{
//!       "platform": "macos-universal", "format": "dmg",
//!       "url": "https://example.com/Dashboard-1.4.0.dmg", "size": 41234567,
//!       "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
//!     }]
//!   }]
//! }"#;
//!
//! // 1. What did the pipeline publish?
//! let feed = Feed::parse(document).unwrap();
//!
//! // 2. What is running here?
//! let install = Install::new("dev.silka.dashboard", Version::new(1, 3, 0))
//!     .platform(Platform::MacosArm64)
//!     .os_version(Version::parse("14.2").unwrap())
//!     .identifier("a-uuid-written-at-first-launch");
//!
//! // 3. Does anything apply? (`None` when the rollout has not reached us.)
//! if let Some(offer) = choose(&feed, &install).unwrap() {
//!     assert_eq!(offer.version(), &Version::new(1, 4, 0));
//!     // 4. Download `offer.url()` through `offer.download()`, which checks the
//!     //    size, then the digest, then the signature — in that order.
//!     let download = offer.download();
//!     assert_eq!(download.expected(), 41_234_567);
//! }
//! ```
//!
//! ## Two deliberate refusals
//!
//! **This crate does not verify signatures.** It computes the digest, it hands
//! over the exact bytes that were signed, and it takes a
//! [`SignatureVerifier`](update::SignatureVerifier) the application implements
//! with a real cryptography crate. Hand-rolling Ed25519 field arithmetic inside
//! a UI framework would produce a routine that looks like security and is not.
//! The digest check it *does* perform is integrity, not authenticity, and the
//! type names say so.
//!
//! **This crate does not write minidumps.**
//! [`crash::write_minidump`] returns
//! [`MinidumpError::Unsupported`](crash::MinidumpError::Unsupported) naming the
//! API it is waiting for — the same convention `silka-platform` uses for every
//! backend it does not have yet. What it *does* write is the metadata around the
//! dump, because that is what makes a dump symbolicatable six months later.
//!
//! ## Zero dependencies, on purpose
//!
//! Except `silka-core`, whose [`recover::on_crash`](silka_core::recover::on_crash)
//! is where a panic report is handed over, nothing here depends on anything.
//! SHA-256, the JSON reader, the version ordering: all of it is arithmetic over
//! bytes. An updater is the one component that cannot be repaired by an update,
//! so the code path that decides whether to replace the application is a code
//! path you can read in an afternoon.

#![warn(missing_docs)]
// Documentation is part of the public contract here for the same reason it is in
// `silka-core`: a broken intra-doc link means a rename silently orphaned a
// reference, and in this crate the references are between the four steps of a
// release.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod crash;
pub mod feed;
pub mod json;
pub mod pending;
pub mod sha256;
pub mod update;
pub mod version;

pub use crash::{CrashContext, CrashReport, MinidumpError};
pub use feed::{Artifact, Delta, Feed, MinimumOs, Os, Platform, Release};
pub use json::Json;
pub use pending::{next_launch, swap_in_place, Discard, NextLaunch, Pending};
pub use sha256::{sha256, Digest, Sha256};
pub use update::{
    applicability, bucket_for, choose, verify, Applicability, Download, Install, Offer,
    SignatureVerifier, VerifyError,
};
pub use version::Version;
