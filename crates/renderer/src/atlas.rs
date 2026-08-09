//! Sisi GPU dari glyph atlas: tekstur, sampler, dan **unggah inkremental**.
//!
//! Aturan yang menentukan apakah teks murah atau mahal (REKOMENDASI §3.2):
//! atlas mask 1024² = 1 MiB, dan mengunggahnya setiap frame berarti membakar
//! bandwidth untuk data yang tidak berubah. Karena itu yang diunggah hanyalah
//! **kotak yang dilaporkan berubah** oleh
//! [`GlyphSource::take_dirty`](rustui_paint::GlyphSource::take_dirty) — biasanya
//! nol byte pada frame kedua dan seterusnya.
//!
//! Dua atlas hidup berdampingan dan keduanya selalu ter-bind:
//!
//! | Atlas | Format tekstur | Isi |
//! |---|---|---|
//! | Mask | `R8Unorm` | cakupan alpha; warnanya datang dari token theme |
//! | Color | `Rgba8Unorm(Srgb)` | emoji berwarna / COLR (jalurnya sudah ada) |
//!
//! Tekstur placeholder 1×1 dibuat sejak awal supaya bind group **selalu**
//! valid — aplikasi tanpa teks tidak membayar apa pun, dan tidak ada jalur
//! kode "pipeline tanpa tekstur" yang harus dijaga terpisah.

use rustui_paint::{AtlasRegion, GlyphFormat, GlyphSource};

/// Satu tekstur atlas beserta ukuran yang sedang ditampungnya.
#[derive(Debug)]
struct AtlasTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    /// Sisi tekstur dalam piksel. `1` berarti masih placeholder.
    size: u32,
}

impl AtlasTexture {
    fn new(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            format,
            size,
        }
    }
}

/// Kedua atlas glyph di GPU, plus sampler-nya.
///
/// `revision` bertambah setiap kali sebuah tekstur dibuat ulang (atlas tumbuh
/// karena penuh). Pipeline memakainya untuk tahu kapan bind group harus
/// dirakit ulang — tanpa itu, bind group akan menunjuk tekstur yang sudah
/// mati dan teks menghilang tanpa error.
#[derive(Debug)]
pub(crate) struct GlyphAtlasGpu {
    mask: AtlasTexture,
    color: AtlasTexture,
    sampler: wgpu::Sampler,
    revision: u64,
}

impl GlyphAtlasGpu {
    /// Atlas kosong (placeholder 1×1) untuk target dengan ruang warna tertentu.
    ///
    /// `srgb_target` menentukan format atlas warna: pada target `*Srgb` shader
    /// menulis nilai linear, jadi tekstur emoji harus ikut di-decode dari sRGB
    /// oleh hardware. Atlas mask tidak terpengaruh — ia menyimpan cakupan
    /// alpha, bukan warna.
    pub(crate) fn new(device: &wgpu::Device, srgb_target: bool) -> Self {
        let color_format = if srgb_target {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        Self {
            mask: AtlasTexture::new(device, "rustui.atlas.mask", wgpu::TextureFormat::R8Unorm, 1),
            color: AtlasTexture::new(device, "rustui.atlas.color", color_format, 1),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("rustui.atlas.sampler"),
                // Clamp: glyph di tepi atlas tidak boleh mengambil texel dari
                // sisi seberang. Padding 1 px antar entri menjaga tetangga.
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                // Linear, bukan nearest: kotak tujuan sudah disetel ke grid
                // piksel fisik sehingga hasilnya identik dengan nearest pada
                // scale bulat — tapi pada scale pecahan (Wayland 1.25) linear
                // menurun dengan anggun alih-alih membuat glyph patah.
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            revision: 0,
        }
    }

    /// Nomor revisi tekstur; berubah = bind group harus dirakit ulang.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// View atlas mask.
    pub(crate) fn mask_view(&self) -> &wgpu::TextureView {
        &self.mask.view
    }

    /// View atlas warna.
    pub(crate) fn color_view(&self) -> &wgpu::TextureView {
        &self.color.view
    }

    /// Sampler bersama kedua atlas.
    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Samakan tekstur GPU dengan atlas sisi-CPU.
    ///
    /// Dipanggil sekali per frame sebelum draw. Biayanya nol byte selama tidak
    /// ada glyph baru; sebesar kotak dirty saat ada glyph baru; dan sebesar
    /// seluruh atlas hanya saat atlas berganti ukuran.
    pub(crate) fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: &mut dyn GlyphSource,
    ) {
        for format in GlyphFormat::ALL {
            let diminta = glyphs.atlas_size(format);
            let bpp = format.bytes_per_pixel();
            let cukup = diminta > 0
                && glyphs.atlas_pixels(format).len() >= (diminta * diminta * bpp) as usize;
            if !cukup {
                // Belum ada atlas (aplikasi tanpa teks) — placeholder 1×1
                // tetap terpasang, dan dirty apa pun tidak berlaku lagi.
                glyphs.take_dirty(format);
                continue;
            }

            let tumbuh = self.texture(format).size != diminta;
            if tumbuh {
                let (label, tex_format) = match format {
                    GlyphFormat::Mask => ("rustui.atlas.mask", self.mask.format),
                    GlyphFormat::Color => ("rustui.atlas.color", self.color.format),
                };
                *self.texture_mut(format) = AtlasTexture::new(device, label, tex_format, diminta);
                self.revision += 1;
            }

            // Tekstur baru = seluruh isinya harus diunggah, apa pun kata
            // dirty region. Kalau tidak, glyph lama akan hilang setelah atlas
            // tumbuh.
            let kotak = match (tumbuh, glyphs.take_dirty(format)) {
                (true, _) => AtlasRegion::new(0, 0, diminta, diminta),
                (false, Some(k)) => k,
                (false, None) => continue,
            };
            let kotak = jepit(kotak, diminta);
            if kotak.is_empty() {
                continue;
            }

            let stride = diminta * bpp;
            let piksel = glyphs.atlas_pixels(format);
            // Offset menunjuk piksel kiri-atas kotak; tiap baris berikutnya
            // maju satu stride penuh. Dengan begitu unggahan parsial tidak
            // butuh salinan sementara sama sekali.
            let offset = (kotak.y * stride + kotak.x * bpp) as u64;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture(format).texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: kotak.x,
                        y: kotak.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                piksel,
                wgpu::TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(kotak.height),
                },
                wgpu::Extent3d {
                    width: kotak.width,
                    height: kotak.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn texture(&self, format: GlyphFormat) -> &AtlasTexture {
        match format {
            GlyphFormat::Mask => &self.mask,
            GlyphFormat::Color => &self.color,
        }
    }

    fn texture_mut(&mut self, format: GlyphFormat) -> &mut AtlasTexture {
        match format {
            GlyphFormat::Mask => &mut self.mask,
            GlyphFormat::Color => &mut self.color,
        }
    }
}

/// Batasi kotak dirty ke dalam atlas — sumber atlas yang cacat tidak boleh
/// membuat wgpu panic di tengah frame (§9.7).
fn jepit(kotak: AtlasRegion, size: u32) -> AtlasRegion {
    if kotak.x >= size || kotak.y >= size {
        return AtlasRegion::EMPTY;
    }
    AtlasRegion::new(
        kotak.x,
        kotak.y,
        kotak.width.min(size - kotak.x),
        kotak.height.min(size - kotak.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kotak_dijepit_ke_dalam_atlas() {
        assert_eq!(
            jepit(AtlasRegion::new(0, 0, 64, 64), 32),
            AtlasRegion::new(0, 0, 32, 32)
        );
        assert_eq!(
            jepit(AtlasRegion::new(30, 30, 8, 8), 32),
            AtlasRegion::new(30, 30, 2, 2)
        );
        assert!(jepit(AtlasRegion::new(40, 0, 8, 8), 32).is_empty());
        assert_eq!(
            jepit(AtlasRegion::new(4, 4, 8, 8), 32),
            AtlasRegion::new(4, 4, 8, 8)
        );
    }
}
