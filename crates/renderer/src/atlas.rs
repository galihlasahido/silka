//! The GPU side of the glyph atlas: textures, sampler, and **incremental
//! uploads**.
//!
//! The rule that decides whether text is cheap or expensive (REKOMENDASI §3.2):
//! a 1024² mask atlas is 1 MiB, and uploading it every frame means burning
//! bandwidth on data that did not change. So the only thing uploaded is the
//! **rect reported as dirty** by
//! [`GlyphSource::take_dirty`](silka_paint::GlyphSource::take_dirty) — usually
//! zero bytes from the second frame onward.
//!
//! Two atlases live side by side and both are always bound:
//!
//! | Atlas | Texture format | Contents |
//! |---|---|---|
//! | Mask | `R8Unorm` | alpha coverage; the color comes from theme tokens |
//! | Color | `Rgba8Unorm(Srgb)` | color emoji / COLR (the path already exists) |
//!
//! A 1×1 placeholder texture is created up front so the bind group is **always**
//! valid — an application without text pays nothing, and there is no separate
//! "pipeline without textures" code path to maintain.

use silka_paint::{AtlasRegion, GlyphFormat, GlyphSource};

/// One atlas texture together with the size it currently holds.
#[derive(Debug)]
struct AtlasTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    /// The texture's side in pixels. `1` means it is still the placeholder.
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

/// Both glyph atlases on the GPU, plus their sampler.
///
/// `revision` increments every time a texture is recreated (the atlas grew
/// because it filled up). The pipeline uses it to know when the bind group must
/// be rebuilt — without it, the bind group would point at a dead texture and
/// text would vanish without an error.
#[derive(Debug)]
pub(crate) struct GlyphAtlasGpu {
    mask: AtlasTexture,
    color: AtlasTexture,
    sampler: wgpu::Sampler,
    revision: u64,
}

impl GlyphAtlasGpu {
    /// An empty atlas (1×1 placeholder) for a target with a given color space.
    ///
    /// `srgb_target` decides the color atlas format: on a `*Srgb` target the
    /// shader writes linear values, so the emoji texture must likewise be
    /// decoded from sRGB by the hardware. The mask atlas is unaffected — it
    /// stores alpha coverage, not color.
    pub(crate) fn new(device: &wgpu::Device, srgb_target: bool) -> Self {
        let color_format = if srgb_target {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        Self {
            mask: AtlasTexture::new(device, "silka.atlas.mask", wgpu::TextureFormat::R8Unorm, 1),
            color: AtlasTexture::new(device, "silka.atlas.color", color_format, 1),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("silka.atlas.sampler"),
                // Clamp: a glyph at the atlas edge must not pull texels from
                // the opposite side. The 1 px padding between entries keeps
                // neighbors out.
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                // Linear, not nearest: the destination box is already snapped
                // to the physical pixel grid, so the result is identical to
                // nearest at integer scales — but at fractional scales
                // (Wayland 1.25) linear degrades gracefully instead of
                // breaking glyphs apart.
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            revision: 0,
        }
    }

    /// The texture revision number; a change means the bind group must be
    /// rebuilt.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The mask atlas view.
    pub(crate) fn mask_view(&self) -> &wgpu::TextureView {
        &self.mask.view
    }

    /// The color atlas view.
    pub(crate) fn color_view(&self) -> &wgpu::TextureView {
        &self.color.view
    }

    /// The sampler shared by both atlases.
    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Bring the GPU textures in sync with the CPU-side atlas.
    ///
    /// Called once per frame before drawing. It costs zero bytes as long as
    /// there are no new glyphs; the size of the dirty rect when there are; and
    /// the size of the whole atlas only when the atlas changes size.
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
                // No atlas yet (an application without text) — the 1×1
                // placeholder stays bound, and any dirty rect no longer
                // applies.
                glyphs.take_dirty(format);
                continue;
            }

            let tumbuh = self.texture(format).size != diminta;
            if tumbuh {
                let (label, tex_format) = match format {
                    GlyphFormat::Mask => ("silka.atlas.mask", self.mask.format),
                    GlyphFormat::Color => ("silka.atlas.color", self.color.format),
                };
                *self.texture_mut(format) = AtlasTexture::new(device, label, tex_format, diminta);
                self.revision += 1;
            }

            // A new texture means the whole thing must be uploaded, whatever
            // the dirty region says. Otherwise the old glyphs would disappear
            // once the atlas grows.
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
            // The offset points at the rect's top-left pixel; every following
            // row advances by one full stride. That way a partial upload needs
            // no temporary copy at all.
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

/// Constrain the dirty rect to the atlas — a malformed atlas source must not
/// make wgpu panic mid-frame (§9.7).
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
