//! Executing one frame: a [`Scene`] into a texture view.
//!
//! Shared by [`crate::WindowSurface`] and [`crate::OffscreenTarget`] so there is
//! exactly **one** implementation of "what a frame is". That matters more than it
//! sounds: the headless path is what golden tests assert on (§9.5), and it is only
//! evidence about the real thing if it is the same code.
//!
//! A frame without layers is one clear pass plus one draw pass, as it always was.
//! A frame with layers becomes a sequence of passes, in this order:
//!
//! ```text
//! clear final target
//! ├─ draw batches before the layer      → final target
//! ├─ draw the layer's contents          → layer texture (cleared first)
//! │  ├─ blur it (dual-Kawase down/up)   → layer texture, in place
//! │  └─ composite it                    → final target (or the enclosing layer)
//! └─ draw batches after the layer       → final target
//! ```
//!
//! Nested layers slot into the same shape, one target per nesting depth. The
//! order is produced by `crate::instance`, on the CPU, where it can be tested
//! without a GPU at all — this module only walks it.

use silka_paint::{GlyphSource, ImageSource, Scene};

use crate::format::clear_color;
use crate::geometry::SurfaceGeometry;
use crate::gpu::Gpu;
use crate::instance::{fill_draw_list, ColorSpace, DrawList};
use crate::layer::LayerStack;
use crate::pipeline::SdfPipeline;

/// Everything needed to turn scenes into pixels on one target format.
#[derive(Debug)]
pub(crate) struct FrameRenderer {
    format: wgpu::TextureFormat,
    sdf: SdfPipeline,
    layers: LayerStack,
    /// The draw list (instances, clip batches, render passes), reused every frame
    /// — the steady state is allocation-free (§3.5: predictable frame times).
    list: DrawList,
}

impl FrameRenderer {
    /// Build the pipelines for one target format.
    ///
    /// Called when a surface is created, never on the first frame: shader
    /// compilation is paid for up front so the first frame does not jank (§3.2).
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            sdf: SdfPipeline::new(device, format),
            layers: LayerStack::new(device, format),
            list: DrawList::default(),
        }
    }

    /// Draw one scene into `target` and submit the work.
    ///
    /// `glyphs` and `images` are the application's atlas sources; the caller never
    /// touches a wgpu type, and this crate never learns what a font or an image
    /// file is.
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        target: &wgpu::TextureView,
        geometry: SurfaceGeometry,
        scene: &Scene,
        glyphs: &mut dyn GlyphSource,
        images: &mut dyn ImageSource,
    ) {
        // The color space follows the target format: a `*Srgb` target does the
        // encoding back in hardware, so the shader must write linear values.
        let space = if self.format.is_srgb() {
            ColorSpace::Linear
        } else {
            ColorSpace::Srgb
        };
        fill_draw_list(
            scene,
            space,
            geometry.scale_factor() as f32,
            glyphs,
            images,
            &mut self.list,
        );
        self.sdf.prepare(
            gpu.device(),
            gpu.queue(),
            geometry.logical_size(),
            &self.list,
            glyphs,
            images,
        );
        self.layers
            .prepare(gpu.device(), geometry, self.list.layer_slots());

        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("silka.frame"),
            });

        // The scene's background is folded into the FIRST pass that touches the
        // final target, so a frame without layers is still exactly one render
        // pass — the same work it was before layers existed. A separate clear
        // pass would cost a full load/store of the surface on tiler GPUs, which
        // is precisely the hardware this framework targets first.
        let latar = clear_color(scene.clear_color(), self.format);
        let mut perlu_clear = true;

        for step in self.list.steps() {
            if !step.is_empty() || step.clear_target {
                // The view borrow is confined to this block so the composite
                // below can take the layer pool mutably again.
                let view = match step.target {
                    None => Some(target),
                    Some(slot) => self.layers.target_view(slot),
                };
                if let Some(view) = view {
                    let load = match step.target {
                        // The final target: clear once, load afterwards.
                        None if perlu_clear => {
                            perlu_clear = false;
                            wgpu::LoadOp::Clear(latar)
                        }
                        // A fresh layer target starts fully transparent: what it
                        // does not draw must show whatever is underneath it.
                        Some(_) if step.clear_target => {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        }
                        _ => wgpu::LoadOp::Load,
                    };
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("silka.frame.pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if !step.is_empty() {
                        self.sdf.draw_batches(
                            &mut pass,
                            &self.list,
                            geometry,
                            step.first_batch,
                            step.last_batch,
                        );
                    }
                }
            }

            if let Some(komposit) = step.composite {
                // Compositing loads what is already on the destination, so the
                // background has to be there first. A scene that opens with a
                // layer is the one case that pays for its own clear pass.
                if komposit.into.is_none() && perlu_clear {
                    bersihkan(&mut encoder, target, latar);
                    perlu_clear = false;
                }
                self.layers
                    .blur(&mut encoder, komposit.source, komposit.blur, geometry);
                self.layers.composite(
                    gpu.device(),
                    gpu.queue(),
                    &mut encoder,
                    target,
                    komposit.source,
                    komposit.into,
                    komposit.bounds,
                    komposit.opacity,
                    geometry.logical_size(),
                );
            }
        }

        // An empty scene is still a frame: the background must be presented, or
        // the window shows whatever the swapchain happened to hold.
        if perlu_clear {
            bersihkan(&mut encoder, target, latar);
        }

        gpu.queue().submit(Some(encoder.finish()));
    }
}

/// A pass that only clears — used when nothing else will.
fn bersihkan(encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, color: wgpu::Color) {
    let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("silka.frame.clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}
