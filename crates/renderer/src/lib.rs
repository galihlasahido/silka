//! # silka-renderer
//!
//! The **wgpu** backend — the only place in the workspace allowed to touch
//! wgpu types (REKOMENDASI §3.2). It implements the `silka-paint` draw
//! commands with a UI-specific SDF shader in the spirit of GPUI:
//!
//! - Rounded rect + **squircle** (G2-continuous superellipse) straight in the
//!   SDF shader; radius/curvature arrive as per-command parameters.
//! - Double ambient + key shadow, border, glyphs from an atlas, monochrome
//!   icons.
//! - In-app blur (dual-Kawase) through a layer/offscreen texture.
//!
//! The BINDING lesson from Impeller: **every shader variant is compiled at
//! build time** — a shader is never generated at runtime (§3.2). Render only
//! when dirty; vsync through the per-platform display link (§3.5).
//!
//! Future alternative backends (vello_hybrid GL, tiny-skia CPU) become sibling
//! crates implementing the very same `silka-paint`.
//!
//! ## What's here (milestones `window-wgpu` + `sdf-shader` + `glyph-gpu-bridge` + `clip-gpu`)
//!
//! The surface foundation: [`Gpu`] (instance/adapter/device/queue, Metal on
//! macOS), [`WindowSurface`] (swapchain, resize, DPI), and a correct
//! sRGB→linear color space conversion.
//!
//! On top of that sits the SDF pipeline (`shaders/sdf.wgsl`), which rasterizes
//! the entire box vocabulary in **a single draw call**:
//!
//! | What differs | How it is expressed |
//! |---|---|
//! | Arc vs squircle | per-instance superellipse exponent (2 vs ≈4) |
//! | Per-corner radius | four `f32` per instance, already scaled CPU-side |
//! | Border | per-instance width; a ring between two SDF isolines |
//! | Ambient + key shadow | two gaussian-blurred instances behind the box |
//! | **Glyph** | textured instance: atlas UV + run color from theme tokens |
//!
//! Because it is all data, there are **no shader variants** and no WGSL is
//! assembled at runtime. Anti-aliasing is derived from screen-space
//! derivatives, so it is correct on 2× Retina and on fractional Wayland scales
//! alike, without any extra parameter.
//!
//! ### Text
//!
//! A `GlyphRun` command becomes a textured quad sampling the glyph atlas.
//! What keeps text crisp and cheap:
//!
//! - **The destination box is snapped to the physical pixel grid** so one texel
//!   lands exactly on one screen pixel (crisp at 2×); subpixel *positioning* is
//!   preserved because it is baked into the bitmap the text layer picked.
//! - **Incremental upload**: only the atlas rects that actually changed are
//!   sent to the GPU — zero bytes on frames whose text did not change.
//! - **One draw call for the whole scene**: text rides in the same command
//!   order as boxes and shadows, so text always sits above its background.
//! - The atlas comes from [`silka_paint::GlyphSource`] — the backend never
//!   mentions `silka-text`, and `silka-text` never mentions wgpu.
//!
//! ### Clip
//!
//! `Command::PushClip`/`PopClip` become **GPU scissor rects**: the scene is
//! split into a list of `(clip rect, instance range)` batches whose order is
//! exactly the command order, and a new batch only opens when the clip rect
//! changes — a UI without clipping stays a single draw call, one scroll view
//! adds two. The rect is used as-is because nested clip intersection is already
//! resolved by `silka-core`; all the backend still maintains is the memory of
//! the parent rect, to be restored on `PopClip`. Converting logical points →
//! physical pixels through [`SurfaceGeometry`] rounds **outward** (content edges
//! are never eaten) and clamps to the surface bounds (a scissor outside those
//! bounds is a wgpu validation error). Batches whose rect is empty are skipped
//! entirely.
//!
//! The same path is available without a window through [`Gpu::headless`] +
//! [`OffscreenTarget`] — the foundation for visual golden/snapshot tests in CI
//! (§9.5), including the "text really does produce pixels" check in
//! `tests/teks.rs`.
//!
//! ## The boundaries being kept
//!
//! This crate's public surface only uses `silka-paint` and `raw-window-handle`
//! types. It does **not** know what winit is, and its callers do **not** need
//! to know what wgpu is. The single door into the wgpu world is
//! [`Gpu::device`], reserved for sibling backend crates.
//!
//! ```no_run
//! use std::sync::Arc;
//! use silka_paint::{Color, Scene, Size};
//! use silka_renderer::{Gpu, SurfaceGeometry};
//!
//! # fn contoh<W: silka_renderer::WindowTarget>(window: Arc<W>) -> Result<(), Box<dyn std::error::Error>> {
//! let geometry = SurfaceGeometry::from_logical(Size::new(1024.0, 720.0), 2.0);
//! let (gpu, mut surface) = Gpu::with_surface(window, geometry)?;
//!
//! // The background color always comes from a theme token, never a literal.
//! let scene = Scene::new(Color::hex(0x1C1C1E));
//! surface.render(&gpu, &scene)?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod atlas;
mod error;
mod format;
mod geometry;
mod gpu;
mod instance;
mod offscreen;
mod pipeline;
mod surface;

pub use error::RendererError;
pub use geometry::SurfaceGeometry;
pub use gpu::{Gpu, WindowTarget};
pub use offscreen::{OffscreenTarget, Rgba8Image};
pub use surface::{FrameOutcome, WindowSurface};
