//! # silka-testing
//!
//! The test harness for the framework — REKOMENDASI §9.5, the gap the design
//! review named as "testing strategy: currently zero".
//!
//! Everything here exists to make one claim checkable: **the UI still looks and
//! behaves the way it did, on every operating system, at 120 fps**. That claim
//! decomposes into four tools, and the crate is four modules:
//!
//! | Module | The question it answers |
//! |---|---|
//! | [`headless`] | Can this scene be drawn without a window? (texture + readback) |
//! | [`golden`] | Does it still look the same? (snapshot files, per preset) |
//! | [`sim`] | Does it still behave the same? (synthetic pointer/keyboard) |
//! | [`mod@bench`] | Is it still fast enough? (frame time with a failing gate) |
//!
//! plus [`matrix`], which insists every visual test runs in **all four cells**
//! of preset × appearance (§2.7), and [`png`]/[`image`]/[`diff`], which are the
//! arithmetic underneath — pure, dependency-free, and unit-tested on their own.
//!
//! ## Why the harness is a crate and not a `#[cfg(test)]` module
//!
//! Before this crate, every gallery page reimplemented its own headless setup:
//! acquire a device, build an offscreen target, remember to skip when there is
//! no GPU, hand-roll a scale factor. Copies drift, and a drifted harness tests
//! a runtime nobody ships. One crate, used as a `dev-dependency`, means the
//! app under test is assembled by [`silka_platform::headless_app`] — the same
//! assembly `run_app` performs — every single time.
//!
//! ## The shape of a visual test
//!
//! ```no_run
//! use silka_testing::{gpu_or_skip, matrix::for_each_case, Simulator, Tolerance};
//! use silka_core::view::fixed;
//!
//! #[test]
//! fn kartu_terlihat_sama_di_setiap_preset() {
//!     let mut gpu = gpu_or_skip!();
//!     for_each_case(|case| {
//!         let mut sim = Simulator::case(case, |_cx| fixed(120.0, 48.0).into());
//!         sim.settle();
//!         let capture = sim.capture(&mut gpu);
//!         case.golden("kartu").tolerance(Tolerance::SHAPES).assert(&capture);
//!     });
//! }
//! ```
//!
//! ## The environment variables it reads
//!
//! | Variable | Effect |
//! |---|---|
//! | `SILKA_GOLDEN=new\|update` | write missing goldens / overwrite all of them |
//! | `SILKA_GOLDEN_TOLERANCE` | per-channel allowance, for a noisier driver |
//! | `SILKA_GOLDEN_RATIO` | fraction of pixels allowed to differ |
//! | `SILKA_REQUIRE_GPU=1` | "no GPU" becomes a failure instead of a skip |
//! | `SILKA_BENCH_ITERATIONS` | frames measured per benchmark |
//! | `SILKA_BENCH_SCALE` | multiplies every frame budget |
//! | `SILKA_BENCH_FORCE=1` | enforce budgets in debug builds too |
//!
//! ## What it deliberately does not do
//!
//! - **It does not compress.** Golden PNGs use DEFLATE stored blocks so the
//!   project owes no compression dependency; see [`png`].
//! - **It does not time the GPU.** [`mod@bench`] measures the CPU frame path, which
//!   is our code; GPU time on a CI runner measures the runner.
//! - **It does not chase bit-exactness.** [`diff::Tolerance`] exists because
//!   three drivers rasterise the same SDF three slightly different ways, and a
//!   suite that demands equality is a suite that gets deleted.

#![deny(missing_docs)]
// Documentation is part of the public contract, so the checks rustdoc offers
// are turned on here rather than left to a reviewer's eye. A broken intra-doc
// link is an error: it means a rename silently orphaned a reference.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod bench;
pub mod diff;
pub mod golden;
pub mod headless;
pub mod image;
pub mod matrix;
pub mod png;
pub mod sim;

pub use bench::{Bench, Budget, Samples};
pub use diff::{compare, visualize, Diff, Tolerance};
pub use golden::{Golden, GoldenFailure, Mode, Outcome};
pub use headless::Headless;
pub use image::Image;
pub use matrix::{for_each_case, Case};
pub use sim::Simulator;

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
