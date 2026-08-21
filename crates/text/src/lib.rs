//! # silka-text
//!
//! A thin wrapper over **cosmic-text** (REKOMENDASI §3.3) — the hardest layer
//! in the whole framework and the #1 killer of new GUI frameworks (§5 failure
//! mode #1). The BINDING contract: **never write your own shaper.**
//!
//! What this crate is responsible for:
//!
//! - **Shaping** through cosmic-text (fontdb + rustybuzz + swash), including
//!   per-platform font fallback, bidi (UAX #9), and ZWJ/color emoji.
//! - **Glyph atlas** — glyphs are rasterized into an atlas and referred to by
//!   id; `silka-paint` speaks only in those ids and never touches a font.
//! - **Measure** — the measurement function for layout leaf nodes, used by
//!   `silka-core` (box constraints) and by Taffy through its measure function
//!   (§3.4).
//!
//! What has to be right from day one (§3.3): subpixel *positioning* (not
//! subpixel AA), caret movement per grapheme cluster (UAX #29), and IME preedit
//! rendered inline. `parley` is the future direction, but not for v1.
//!
//! ## The boundary this crate keeps
//!
//! cosmic-text types **never appear in this crate's public API**. Callers speak
//! in [`TextStyle`], [`TextConstraints`], [`TextMeasure`], [`TextLayout`], and
//! `silka_paint::GlyphRun`. That way, moving to `parley` later is work confined
//! to this crate alone, and widget code still has no idea what a font is
//! (§3.2, §3.3).
//!
//! ## Usage flow
//!
//! ```
//! use silka_paint::{Color, Point, Scene};
//! use silka_text::{TextConstraints, TextEngine, TextStyle};
//!
//! // One engine for the whole application. `bundled_only` = no system fonts,
//! // used by tests/CI so results stay deterministic.
//! let mut teks = TextEngine::bundled_only();
//! teks.set_scale_factor(2.0); // Retina
//!
//! // Styles are always built from theme tokens, never from literal numbers.
//! let gaya = TextStyle::new().size(17.0);
//!
//! // 1. Measure — this is what the layout system uses (box constraints, §3.4).
//! let ukuran = teks.measure("Halo, dunia", &gaya, TextConstraints::width(280.0));
//! assert!(ukuran.width() > 0.0 && ukuran.line_count == 1);
//!
//! // 2. Draw — the result is a `GlyphRun` command holding atlas ids, not fonts.
//! let mut scene = Scene::new(Color::hex(0x1C1C1E));
//! teks.draw(
//!     &mut scene,
//!     "Halo, dunia",
//!     &gaya,
//!     TextConstraints::width(280.0),
//!     Point::new(24.0, 24.0),
//!     Color::WHITE,
//! );
//! assert_eq!(scene.len(), 1);
//! ```
//!
//! ## What the backend sees
//!
//! A `GlyphRun` command carries only **atlas ids + logical destination rects**.
//! The backend turns those ids into textures through the
//! [`silka_paint::GlyphSource`] trait, implemented by [`TextEngine`] (and
//! [`GlyphCache`]) — that trait is the entire surface the backend sees:
//!
//! ```
//! use silka_paint::{GlyphFormat, GlyphSource};
//! use silka_text::TextEngine;
//!
//! let mut teks = TextEngine::bundled_only();
//! # let gaya = silka_text::TextStyle::new();
//! # let l = teks.layout("Halo", &gaya, silka_text::TextConstraints::UNBOUNDED);
//! # let run = teks.rasterize(&l, silka_paint::Point::ZERO, silka_paint::Color::WHITE);
//!
//! // What the backend does every frame — without ever saying "font":
//! let sisi = teks.atlas_size(GlyphFormat::Mask);
//! if let Some(kotak) = teks.take_dirty(GlyphFormat::Mask) {
//!     let _piksel = teks.atlas_pixels(GlyphFormat::Mask); // upload just this rect
//!     let _uv = kotak.uv(sisi);
//! }
//! let _letak = teks.placement(run.glyphs[0].image); // id → rect in the atlas
//! ```
//!
//! There are no GPU types here: today's wgpu backend and a later GL/CPU backend
//! read the same source (§3.2, §5 failure mode #7). Conversely, the backend
//! does not need to know this crate exists — it only holds a `&mut dyn
//! GlyphSource`, so moving to `parley` later never touches the renderer.
//!
//! ## Not here yet (technical debt we know about)
//!
//! - Inter's `opsz` (optical size) axis is not yet set automatically per font
//!   size; what does work is the `wght` axis through variable font weight
//!   (§3.6).
//! - Rich text (several styles in one paragraph) and automatic ellipsis; so far
//!   only `max_lines` plus the `overflowed` flag exist as the foundation.
//! - A selection range spanning lines does not yet highlight the line break
//!   itself (visible once multi-line `text_area` lands); what is already
//!   correct is the highlight per visual segment within each line.
//!
//! What **does** exist and used to be listed as debt: editing, per-grapheme
//! carets, and IME preedit live in [`edit`] (a pure model, no pixels), with
//! their geometry — [`TextLayout::hit`], [`TextLayout::caret`], and
//! [`TextLayout::selection_rects`] — in [`layout`]. Both are used by
//! `text_field` (KOMPONEN.md Tier 2).

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
pub mod cache;
pub mod edit;
pub mod engine;
pub mod font;
pub mod layout;
mod lru;
pub mod measure;
pub mod style;

pub use atlas::{AtlasFormat, AtlasRect, GlyphAtlas};
pub use cache::{FontId, GlyphCache, GlyphImage, GlyphKey, GlyphLookup, RasterGlyph, SubpixelBin};
pub use edit::{Movement, Preedit, Selection, TextEdit};
pub use engine::TextEngine;
pub use font::{FontOptions, BUNDLED_UI_FONT};
pub use layout::{Caret, LineMetrics, TextLayout};
pub use measure::{TextConstraints, TextMeasure};
pub use style::{FontFamily, FontWeight, TextAlign, TextStyle, TextWrap};

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
