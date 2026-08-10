//! **Headless** rendering: the same scene, without a window.
//!
//! Needed now rather than later (REKOMENDASI §9.5): a claim of "stability" only
//! means something if there are visual golden/snapshot tests and frame-time
//! benchmarks that can run in CI without a display server. The draw path is
//! **exactly** the one [`crate::WindowSurface`] uses — same SDF pipeline, same
//! sRGB format, same blending — so what is tested headless is what users see.

use silka_paint::{GlyphSource, NoGlyphs, Scene, Size};

use crate::error::RendererError;
use crate::geometry::SurfaceGeometry;
use crate::gpu::Gpu;
use crate::instance::{fill_draw_list, ColorSpace, DrawList};
use crate::pipeline::SdfPipeline;

/// The headless target format: the same class as a window swapchain (sRGB), so
/// the color space conversion is exercised too rather than bypassed.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Texture rows must be copied in multiples of 256 bytes (a WebGPU rule).
const ROW_ALIGNMENT: u32 = 256;

/// An 8-bit RGBA image produced by a headless render, already in **sRGB** space
/// (its byte values can be compared directly against color tokens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Rgba8Image {
    /// Width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// All pixels, four bytes per pixel, row by row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// A single `[r, g, b, a]` pixel. Out of bounds returns transparent.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }
}

/// An offscreen texture that can receive any [`Scene`].
#[derive(Debug)]
pub struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    geometry: SurfaceGeometry,
    padded_row: u32,
    sdf: SdfPipeline,
    list: DrawList,
}

impl OffscreenTarget {
    /// Create a target with a given geometry (physical size + scale factor).
    pub fn new(gpu: &Gpu, geometry: SurfaceGeometry) -> Result<Self, RendererError> {
        if !geometry.is_renderable() {
            return Err(RendererError::SurfaceUnsupported);
        }
        let width = geometry.physical_width();
        let height = geometry.physical_height();

        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("silka.offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let padded_row = padded_row_bytes(width);
        let readback = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("silka.offscreen.readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            texture,
            view,
            readback,
            geometry,
            padded_row,
            sdf: SdfPipeline::new(gpu.device(), FORMAT),
            list: DrawList::default(),
        })
    }

    /// The size in logical points — the same meaning as
    /// [`crate::WindowSurface::logical_size`].
    pub fn logical_size(&self) -> Size {
        self.geometry.logical_size()
    }

    /// Draw one scene, then read the result back to the CPU.
    ///
    /// Synchronous on purpose: this is a testing tool, not an application's
    /// frame path. Without an atlas source, `GlyphRun` commands produce no
    /// pixels — see [`OffscreenTarget::render_with_glyphs`].
    pub fn render(&mut self, gpu: &Gpu, scene: &Scene) -> Result<Rgba8Image, RendererError> {
        self.render_with_glyphs(gpu, scene, &mut NoGlyphs)
    }

    /// Draw one scene **including its text**, then read the result back to the
    /// CPU.
    ///
    /// This is the golden/snapshot test path for text (§9.5): the same atlas a
    /// window would use is uploaded to a texture, and the final result can be
    /// counted pixel by pixel without a display server.
    pub fn render_with_glyphs(
        &mut self,
        gpu: &Gpu,
        scene: &Scene,
        glyphs: &mut dyn GlyphSource,
    ) -> Result<Rgba8Image, RendererError> {
        fill_draw_list(
            scene,
            ColorSpace::Linear,
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

        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("silka.offscreen.frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("silka.offscreen.frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(crate::format::clear_color(
                            scene.clear_color(),
                            FORMAT,
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

        let width = self.geometry.physical_width();
        let height = self.geometry.physical_height();
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue().submit(Some(encoder.finish()));

        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        gpu.device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| RendererError::DeviceUnavailable(e.to_string()))?;

        let baris = (width * 4) as usize;
        let mut pixels = Vec::with_capacity(baris * height as usize);
        {
            let view = self.readback.slice(..).get_mapped_range();
            for y in 0..height as usize {
                let mulai = y * self.padded_row as usize;
                pixels.extend_from_slice(&view[mulai..mulai + baris]);
            }
        }
        self.readback.unmap();

        Ok(Rgba8Image {
            width,
            height,
            pixels,
        })
    }
}

/// The length of one copied row after rounding up to a multiple of 256 bytes.
fn padded_row_bytes(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baris_dibulatkan_ke_kelipatan_256() {
        assert_eq!(padded_row_bytes(64), 256);
        assert_eq!(padded_row_bytes(65), 512);
        assert_eq!(padded_row_bytes(128), 512);
        assert_eq!(padded_row_bytes(1), 256);
    }

    #[test]
    fn piksel_di_luar_batas_aman() {
        let img = Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert_eq!(img.pixel(1, 0), [5, 6, 7, 8]);
        assert_eq!(img.pixel(2, 0), [0, 0, 0, 0]);
        assert_eq!(img.pixel(0, 9), [0, 0, 0, 0]);
    }
}
