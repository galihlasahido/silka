//! Surface window: swapchain, resize/DPI, dan eksekusi satu [`Scene`].

use rustui_paint::{GlyphSource, NoGlyphs, Scene, Size};

use crate::error::RendererError;
use crate::format::{choose_alpha_mode, choose_surface_format, clear_color};
use crate::geometry::SurfaceGeometry;
use crate::gpu::Gpu;
use crate::instance::{fill_draw_list, ColorSpace, DrawList};
use crate::pipeline::SdfPipeline;

/// Hasil satu upaya menggambar frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// Frame digambar dan dipresentasikan.
    Presented,
    /// Frame sengaja dilewati: window kosong/minimal, tertutup window lain,
    /// atau swapchain sedang timeout. Bukan kesalahan — scheduler cukup
    /// menunggu event berikutnya (§3.5: render hanya saat dirty).
    Skipped,
}

/// Swapchain untuk satu window.
///
/// API-nya sengaja bebas tipe wgpu: `rustui-platform` cukup meneruskan ukuran
/// fisik dari winit dan sebuah [`Scene`].
#[derive(Debug)]
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    geometry: SurfaceGeometry,
    configured: bool,
    sdf: SdfPipeline,
    /// Daftar gambar (instance + batch clip) yang dipakai ulang tiap frame —
    /// steady-state bebas alokasi (§3.5: frame time prediktabel).
    list: DrawList,
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
            // AutoVsync menurunkan diri ke mode yang didukung platform;
            // di macOS ini memberi presentasi yang sinkron dengan
            // CVDisplayLink, termasuk ProMotion 120 Hz.
            present_mode: wgpu::PresentMode::AutoVsync,
            // 2 = keseimbangan latensi/throughput untuk UI (dokumentasi wgpu).
            desired_maximum_frame_latency: 2,
            alpha_mode: choose_alpha_mode(&caps.alpha_modes),
            view_formats: Vec::new(),
        };

        // Pipeline dibuat sekarang, bukan saat frame pertama: kompilasi shader
        // dibayar di muka supaya frame pertama tidak jank (§3.2).
        let sdf = SdfPipeline::new(gpu.device(), format);

        let mut this = Self {
            surface,
            config,
            geometry,
            configured: false,
            sdf,
            list: DrawList::default(),
        };
        this.reconfigure(gpu);
        Ok(this)
    }

    /// Geometri surface saat ini.
    pub fn geometry(&self) -> SurfaceGeometry {
        self.geometry
    }

    /// Ukuran dalam poin logis — inilah yang diteruskan ke layout.
    pub fn logical_size(&self) -> Size {
        self.geometry.logical_size()
    }

    /// Scale factor window.
    pub fn scale_factor(&self) -> f64 {
        self.geometry.scale_factor()
    }

    /// Terapkan ukuran fisik baru (event `Resized` winit).
    ///
    /// Ukuran 0×0 (window diminimalkan) diterima tanpa mengonfigurasi
    /// swapchain — wgpu menolak dimensi nol.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        let baru = self.geometry.with_physical_size(width, height);
        if baru == self.geometry && self.configured {
            return;
        }
        self.geometry = baru;
        self.reconfigure(gpu);
    }

    /// Terapkan scale factor baru (event `ScaleFactorChanged`).
    ///
    /// Tidak menyentuh swapchain: winit selalu menyusulkan `Resized` dengan
    /// ukuran fisik yang benar. Yang berubah di sini hanyalah pembagi
    /// poin-logis, supaya frame berikutnya layout-nya benar.
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.geometry = self.geometry.with_scale_factor(scale_factor);
    }

    /// Konfigurasi ulang swapchain dari geometri saat ini.
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

    /// Gambar satu frame **tanpa teks**.
    ///
    /// Perintah `GlyphRun` di dalam scene tidak menghasilkan piksel apa pun:
    /// tanpa sumber atlas tidak ada bitmap yang bisa digambar. Untuk teks,
    /// pakai [`WindowSurface::render_with_glyphs`].
    pub fn render(&mut self, gpu: &Gpu, scene: &Scene) -> Result<FrameOutcome, RendererError> {
        self.render_with_glyphs(gpu, scene, &mut NoGlyphs)
    }

    /// Gambar satu frame beserta teksnya.
    ///
    /// Seluruh perintah (quad, border, bayangan, **dan glyph**) dieksekusi
    /// lewat satu pipeline SDF dalam **satu draw call** — perbedaan bentuknya
    /// (arc/squircle, ada/tidak border, blur, bertekstur/tidak) adalah data
    /// instance, bukan varian shader. Karena semuanya satu draw call, urutan
    /// perintah scene sekaligus menjadi urutan gambar: teks di atas latarnya,
    /// tidak pernah tertimpa.
    ///
    /// `glyphs` biasanya `&mut TextEngine`. Kontraknya tetap: pemanggil tidak
    /// pernah menyentuh tipe wgpu, dan backend tidak pernah tahu apa itu font.
    pub fn render_with_glyphs(
        &mut self,
        gpu: &Gpu,
        scene: &Scene,
        glyphs: &mut dyn GlyphSource,
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

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            // Suboptimal: dipakai sekali lagi, lalu ditata ulang untuk frame
            // berikutnya — inilah yang terjadi saat window sedang di-drag
            // antar monitor dengan DPI berbeda.
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

        // Ruang warna ditentukan format target: `*Srgb` melakukan encoding
        // balik di hardware, jadi shader harus menulis nilai linear.
        let space = if self.config.format.is_srgb() {
            ColorSpace::Linear
        } else {
            ColorSpace::Srgb
        };
        fill_draw_list(
            scene,
            space,
            self.geometry.scale_factor() as f32,
            glyphs,
            &mut self.list,
        );
        self.sdf.prepare(
            gpu.device(),
            gpu.queue(),
            self.geometry.logical_size(),
            &self.list,
            glyphs,
        );

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rustui.frame"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rustui.frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color(
                            scene.clear_color(),
                            self.config.format,
                        )),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.sdf.draw(&mut pass, &self.list, self.geometry);
        }

        gpu.queue().submit(Some(encoder.finish()));
        frame.present();
        Ok(FrameOutcome::Presented)
    }
}
