//! The window surface: swapchain, resize/DPI, and executing one [`Scene`].

use silka_paint::{GlyphSource, ImageSource, NoGlyphs, NoImages, Scene, Size};

use crate::error::RendererError;
use crate::format::{choose_alpha_mode, choose_surface_format};
use crate::frame::FrameRenderer;
use crate::geometry::SurfaceGeometry;
use crate::gpu::Gpu;

/// The result of one attempt to draw a frame.
///
/// ```
/// use silka_renderer::FrameOutcome;
///
/// fn frame_was_shown(outcome: FrameOutcome) -> bool {
///     // `Skipped` is not an error: a minimized or occluded window simply has
///     // nothing to present, and the scheduler goes back to waiting.
///     matches!(outcome, FrameOutcome::Presented)
/// }
///
/// assert!(frame_was_shown(FrameOutcome::Presented));
/// assert!(!frame_was_shown(FrameOutcome::Skipped));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// The frame was drawn and presented.
    Presented,
    /// The frame was deliberately skipped: the window is empty/minimized,
    /// occluded by another window, or the swapchain is timing out. Not an
    /// error — the scheduler simply waits for the next event (§3.5: render
    /// only when dirty).
    Skipped,
}

/// The swapchain for one window.
///
/// Its API is deliberately free of wgpu types: `silka-platform` only forwards
/// the physical size from winit and a [`Scene`].
///
/// ```no_run
/// use std::sync::Arc;
/// use silka_paint::{Color, Scene, Size};
/// use silka_renderer::{FrameOutcome, Gpu, SurfaceGeometry, WindowTarget};
///
/// fn run<W: WindowTarget>(window: Arc<W>) -> Result<(), Box<dyn std::error::Error>> {
///     let geometry = SurfaceGeometry::from_logical(Size::new(1024.0, 720.0), 2.0);
///     let (gpu, mut surface) = Gpu::with_surface(window, geometry)?;
///
///     // The shell forwards what winit reports; no wgpu type crosses over.
///     surface.resize(&gpu, 2048, 1440);
///     surface.set_scale_factor(2.0);
///
///     // Draw only when something is dirty (REKOMENDASI §3.5).
///     if surface.render(&gpu, &Scene::new(Color::hex(0x1C1C1E)))? == FrameOutcome::Skipped {
///         // Minimized or occluded: go back to waiting for an event.
///     }
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    geometry: SurfaceGeometry,
    configured: bool,
    /// The pipelines, the layer pool, and the reused draw list — the same frame
    /// implementation the headless path uses, so what a golden test asserts is
    /// what a user sees.
    frame: FrameRenderer,
}

impl WindowSurface {
    pub(crate) fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        geometry: SurfaceGeometry,
    ) -> Result<Self, RendererError> {
        let caps = surface.get_capabilities(gpu.adapter());
        let format =
            choose_surface_format(&caps.formats).ok_or(RendererError::SurfaceUnsupported)?;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: geometry.physical_width().max(1),
            height: geometry.physical_height().max(1),
            // AutoVsync degrades to whatever mode the platform supports; on
            // macOS this gives presentation synchronized with CVDisplayLink,
            // including 120 Hz ProMotion.
            present_mode: wgpu::PresentMode::AutoVsync,
            // 2 = the latency/throughput balance for UI (per the wgpu docs).
            desired_maximum_frame_latency: 2,
            alpha_mode: choose_alpha_mode(&caps.alpha_modes),
            view_formats: Vec::new(),
        };

        // The pipelines are built now, not on the first frame: shader compilation
        // is paid for up front so the first frame does not jank (§3.2).
        let frame = FrameRenderer::new(gpu.device(), format);

        let mut this = Self {
            surface,
            config,
            geometry,
            configured: false,
            frame,
        };
        this.reconfigure(gpu);
        Ok(this)
    }

    /// The surface's current geometry.
    pub fn geometry(&self) -> SurfaceGeometry {
        self.geometry
    }

    /// The size in logical points — this is what gets handed to layout.
    pub fn logical_size(&self) -> Size {
        self.geometry.logical_size()
    }

    /// The window's scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.geometry.scale_factor()
    }

    /// Apply a new physical size (winit's `Resized` event).
    ///
    /// A 0×0 size (a minimized window) is accepted without configuring the
    /// swapchain — wgpu rejects zero dimensions.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        let baru = self.geometry.with_physical_size(width, height);
        if baru == self.geometry && self.configured {
            return;
        }
        self.geometry = baru;
        self.reconfigure(gpu);
    }

    /// Apply a new scale factor (the `ScaleFactorChanged` event).
    ///
    /// This does not touch the swapchain: winit always follows up with a
    /// `Resized` carrying the correct physical size. All that changes here is
    /// the logical-point divisor, so the next frame lays out correctly.
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.geometry = self.geometry.with_scale_factor(scale_factor);
    }

    /// Reconfigure the swapchain from the current geometry.
    pub fn reconfigure(&mut self, gpu: &Gpu) {
        if !self.geometry.is_renderable() {
            self.configured = false;
            return;
        }
        self.config.width = self.geometry.physical_width();
        self.config.height = self.geometry.physical_height();
        self.surface.configure(gpu.device(), &self.config);
        self.configured = true;
    }

    /// Draw one frame **without text**.
    ///
    /// `GlyphRun` commands in the scene produce no pixels at all: without an
    /// atlas source there is no bitmap to draw. For text, use
    /// [`WindowSurface::render_with_glyphs`].
    pub fn render(&mut self, gpu: &Gpu, scene: &Scene) -> Result<FrameOutcome, RendererError> {
        self.render_with_glyphs(gpu, scene, &mut NoGlyphs)
    }

    /// Draw one frame, text included.
    ///
    /// Every command (quads, borders, shadows, glyphs, **lines**, and **bitmaps**)
    /// runs through the one SDF pipeline in **a single draw call** — the
    /// differences in shape (arc/squircle, border or not, blur, textured or not,
    /// transformed or not) are instance data, not shader variants. Because it is
    /// all one draw call, the scene's command order doubles as the draw order:
    /// text sits above its background and is never painted over.
    ///
    /// Layers ([`silka_paint::Command::PushLayer`]) are the one exception, and a
    /// deliberate one: they add a render pass per layer, because that is what
    /// "render this subtree to a texture, then blur it" means.
    ///
    /// `glyphs` is usually `&mut TextEngine`. The contract still holds: the
    /// caller never touches a wgpu type, and the backend never knows what a
    /// font is. Images need their own source — see
    /// [`WindowSurface::render_with_sources`].
    pub fn render_with_glyphs(
        &mut self,
        gpu: &Gpu,
        scene: &Scene,
        glyphs: &mut dyn GlyphSource,
    ) -> Result<FrameOutcome, RendererError> {
        self.render_with_sources(gpu, scene, glyphs, &mut NoImages)
    }

    /// Draw one frame with **both** atlas sources: text and bitmaps.
    ///
    /// `images` is usually the application's [`silka_paint::ImageAtlas`]. Without
    /// it, `Command::Image` produces no pixels at all — the same negative-control
    /// behaviour a missing glyph source has, and for the same reason: drawing
    /// nothing is honest, drawing garbage is not.
    pub fn render_with_sources(
        &mut self,
        gpu: &Gpu,
        scene: &Scene,
        glyphs: &mut dyn GlyphSource,
        images: &mut dyn ImageSource,
    ) -> Result<FrameOutcome, RendererError> {
        if !self.geometry.is_renderable() {
            return Ok(FrameOutcome::Skipped);
        }
        if !self.configured {
            self.reconfigure(gpu);
            if !self.configured {
                return Ok(FrameOutcome::Skipped);
            }
        }

        let surface_frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            // Suboptimal: used once more, then reconfigured for the next frame
            // — this is what happens while a window is being dragged between
            // monitors with different DPI.
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.configured = false;
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure(gpu);
                return Ok(FrameOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(FrameOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.reconfigure(gpu);
                return Ok(FrameOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.configured = false;
                return Err(RendererError::SurfaceLost);
            }
        };

        let view = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.frame
            .render(gpu, &view, self.geometry, scene, glyphs, images);
        surface_frame.present();
        Ok(FrameOutcome::Presented)
    }
}
