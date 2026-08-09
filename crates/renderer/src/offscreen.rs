//! Rendering **headless**: scene yang sama, tanpa window.
//!
//! Dibutuhkan sejak sekarang, bukan nanti (REKOMENDASI §9.5): klaim
//! "kestabilan" hanya berarti kalau ada golden/snapshot test visual dan
//! benchmark frame-time yang bisa jalan di CI tanpa server tampilan. Jalur
//! gambarnya **persis sama** dengan [`crate::WindowSurface`] — pipeline SDF,
//! format sRGB, dan blending yang sama — sehingga apa yang diuji headless
//! memang yang dilihat pengguna.

use rustui_paint::{GlyphSource, NoGlyphs, Scene, Size};

use crate::error::RendererError;
use crate::geometry::SurfaceGeometry;
use crate::gpu::Gpu;
use crate::instance::{fill_draw_list, ColorSpace, DrawList};
use crate::pipeline::SdfPipeline;

/// Format target headless: sama kelasnya dengan swapchain window (sRGB),
/// supaya konversi ruang warna diuji juga, bukan dilewati.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Baris tekstur harus disalin dengan kelipatan 256 byte (aturan WebGPU).
const ROW_ALIGNMENT: u32 = 256;

/// Gambar RGBA 8-bit hasil render headless, sudah dalam ruang **sRGB**
/// (angka byte-nya bisa dibandingkan langsung dengan token warna).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Rgba8Image {
    /// Lebar dalam piksel fisik.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Tinggi dalam piksel fisik.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Seluruh piksel, empat byte per piksel, baris demi baris.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Satu piksel `[r, g, b, a]`. Di luar batas mengembalikan transparan.
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

/// Tekstur di luar layar yang bisa menerima [`Scene`] apa pun.
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
    /// Buat target dengan geometri tertentu (ukuran fisik + scale factor).
    pub fn new(gpu: &Gpu, geometry: SurfaceGeometry) -> Result<Self, RendererError> {
        if !geometry.is_renderable() {
            return Err(RendererError::SurfaceUnsupported);
        }
        let width = geometry.physical_width();
        let height = geometry.physical_height();

        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("rustui.offscreen"),
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
            label: Some("rustui.offscreen.readback"),
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

    /// Ukuran dalam poin logis — sama artinya dengan
    /// [`crate::WindowSurface::logical_size`].
    pub fn logical_size(&self) -> Size {
        self.geometry.logical_size()
    }

    /// Gambar satu scene lalu baca hasilnya kembali ke CPU.
    ///
    /// Sinkron dengan sengaja: ini alat uji, bukan jalur frame aplikasi.
    /// Tanpa sumber atlas, perintah `GlyphRun` tidak menghasilkan piksel —
    /// lihat [`OffscreenTarget::render_with_glyphs`].
    pub fn render(&mut self, gpu: &Gpu, scene: &Scene) -> Result<Rgba8Image, RendererError> {
        self.render_with_glyphs(gpu, scene, &mut NoGlyphs)
    }

    /// Gambar satu scene **beserta teksnya** lalu baca hasilnya ke CPU.
    ///
    /// Inilah jalur golden/snapshot test teks (§9.5): atlas yang sama yang
    /// dipakai window diunggah ke tekstur, dan hasil akhirnya bisa dihitung
    /// piksel demi piksel tanpa server tampilan.
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
                label: Some("rustui.offscreen.frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rustui.offscreen.frame"),
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

/// Panjang satu baris salinan setelah dibulatkan ke kelipatan 256 byte.
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
