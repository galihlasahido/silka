//! # silka-paint
//!
//! A thin draw-command abstraction for the whole framework (REKOMENDASI §3.2).
//!
//! This crate defines the *vocabulary* for drawing UI — not how to execute it
//! on the GPU:
//!
//! - **Rounded rect / squircle** — corner geometry (Apple-style continuous
//!   corners) is a *parameter* of the draw command that is passed through to
//!   the shader, not a constant (REKOMENDASI §2.7, §3.6). The Cupertino preset
//!   sends squircles, the Tailwind preset sends plain arcs. See
//!   [`CornerStyle`].
//! - **Glyphs** — referenced through atlas ids owned by `silka-text`.
//! - **Double shadows** (HIG-style ambient + key) and **blur** (dual-Kawase for
//!   materials) — these need layer/offscreen-texture support in the render
//!   graph.
//!
//! ## BINDING contract
//!
//! The public API of this crate **must not expose wgpu types** (or any other
//! graphics API). Widget code speaks only in this crate's draw commands;
//! `silka-renderer` (wgpu) is just one implementation. That way a new backend
//! (GL/CPU/BSD) can be added later in a single place without rewriting the
//! framework (REKOMENDASI §5 failure mode #7).
//!
//! ## Status
//!
//! The vocabulary that exists today: color (with a correct sRGB→linear color
//! space conversion), logical-point geometry, corner geometry as a parameter,
//! and a [`Scene`] holding a list of [`Command`]s. Rasterizing `Command`s
//! themselves belongs to the SDF shader milestone; today's backend only
//! executes the [`Scene::clear_color`] background color.
//!
//! ```
//! use silka_paint::{Color, Corners, CornerStyle, Quad, Rect, Scene};
//!
//! let mut scene = Scene::new(Color::hex(0x1C1C1E));
//! scene.push(
//!     Quad::new(Rect::new(24.0, 24.0, 180.0, 96.0))
//!         .background(Color::hex(0x2C2C2E))
//!         // The corner shape comes from a theme token, not from a literal here.
//!         .corners(Corners::uniform(14.0, CornerStyle::squircle()))
//!         .normalized(),
//! );
//! assert_eq!(scene.len(), 1);
//! ```

#![warn(missing_docs)]
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

pub mod atlas;
pub mod color;
pub mod corner;
pub mod geometry;
pub mod glyph;
pub mod scene;
pub mod shadow;

pub use atlas::{AtlasRegion, GlyphFormat, GlyphPlacement, GlyphSource, NoGlyphs};
pub use color::{linear_to_srgb, srgb_to_linear, Color};
pub use corner::{CornerRadii, CornerStyle, Corners};
pub use geometry::{Insets, Point, Rect, Size};
pub use glyph::{Glyph, GlyphImageId, GlyphRun};
pub use scene::{Command, Quad, Scene, ShadowQuad};
pub use shadow::{Shadow, ShadowPair};

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
