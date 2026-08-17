//! The GPU side of the **image** atlas: one texture, incremental uploads.
//!
//! Deliberately the same shape as [`crate::atlas`], down to the revision
//! counter, because the discipline that keeps text cheap keeps images cheap for
//! the same reason: a 1024² RGBA atlas is 4 MiB, and re-uploading it every frame
//! would burn bandwidth on pixels that did not change. Only the rect
//! [`ImageSource::take_dirty`] reports is sent — usually nothing at all from the
//! second frame onward.
//!
//! One atlas, one binding: photographs, avatars, and monochrome icons all come
//! from here, which is what lets an icon beside a label stay inside the same
//! single draw call as the label.

use silka_paint::{AtlasRegion, ImageSource};

/// Bytes per pixel in the image atlas (RGBA8, straight alpha).
const BYTES_PER_PIXEL: u32 = 4;

/// The image atlas on the GPU.
///
/// `revision` increments whenever the texture is recreated (the CPU-side atlas
/// grew and repacked itself). The pipeline compares it to know when the bind
/// group must be rebuilt — without it the bind group would point at a dead
/// texture and every image would vanish without an error.
#[derive(Debug)]
pub(crate) struct ImageAtlasGpu {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    /// The texture's side in pixels; `1` means it is still the placeholder.
    size: u32,
    revision: u64,
}

impl ImageAtlasGpu {
    /// An empty atlas: a 1×1 placeholder so the bind group is **always** valid.
    ///
    /// An application without images pays one texel and there is no separate
    /// "pipeline without an image texture" code path to maintain.
    ///
    /// `srgb_target` decides the format for the same reason it does for the color
    /// glyph atlas: on a `*Srgb` target the shader writes linear values, so the
    /// hardware must decode the atlas from sRGB on read.
    pub(crate) fn new(device: &wgpu::Device, srgb_target: bool) -> Self {
        let format = if srgb_target {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let (texture, view) = buat_tekstur(device, format, 1);
        Self {
            texture,
            view,
            format,
            size: 1,
            revision: 0,
        }
    }

    /// The texture revision; a change means the bind group must be rebuilt.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The atlas view.
    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Bring the GPU texture in sync with the CPU-side atlas.
    ///
    /// Costs zero bytes while no image was added, the size of the dirty rect when
    /// one was, and the whole atlas only when the atlas itself changed size.
    pub(crate) fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &mut dyn ImageSource,
    ) {
        let diminta = images.atlas_size();
        let cukup = diminta > 0
            && images.atlas_pixels().len() >= (diminta * diminta * BYTES_PER_PIXEL) as usize;
        if !cukup {
            // No atlas yet (an application without images) — the placeholder
            // stays bound, and any dirty rect no longer applies.
            images.take_dirty();
            return;
        }

        let tumbuh = self.size != diminta;
        if tumbuh {
            let (texture, view) = buat_tekstur(device, self.format, diminta);
            self.texture = texture;
            self.view = view;
            self.size = diminta;
            self.revision += 1;
        }

        // A new texture must be filled completely, whatever the dirty rect says:
        // the CPU atlas repacked itself, so every old entry is somewhere else now.
        let kotak = match (tumbuh, images.take_dirty()) {
            (true, _) => AtlasRegion::new(0, 0, diminta, diminta),
            (false, Some(k)) => k,
            (false, None) => return,
        };
        let kotak = jepit(kotak, diminta);
        if kotak.is_empty() {
            return;
        }

        let stride = diminta * BYTES_PER_PIXEL;
        let piksel = images.atlas_pixels();
        // The offset points at the rect's top-left pixel and every following row
        // advances by a full stride, so a partial upload needs no temporary copy.
        let offset = (kotak.y * stride + kotak.x * BYTES_PER_PIXEL) as u64;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
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

fn buat_tekstur(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("silka.atlas.image"),
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
    (texture, view)
}

/// Constrain the dirty rect to the atlas — a malformed source must not make wgpu
/// panic mid-frame (§9.7).
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
            jepit(AtlasRegion::new(0, 0, 512, 512), 256),
            AtlasRegion::new(0, 0, 256, 256)
        );
        assert_eq!(
            jepit(AtlasRegion::new(250, 250, 16, 16), 256),
            AtlasRegion::new(250, 250, 6, 6)
        );
        assert!(jepit(AtlasRegion::new(300, 0, 8, 8), 256).is_empty());
    }
}
