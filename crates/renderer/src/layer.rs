//! Offscreen layer targets, the blur chain, and compositing.
//!
//! This is the GPU half of [`silka_paint::Command::PushLayer`]: a texture to draw
//! a subtree into, a dual-Kawase down/up chain to blur it, and a pipeline to
//! composite the result back over its parent with a group opacity.
//!
//! Two deliberate simplifications, both recorded rather than hidden:
//!
//! 1. **A layer target is the size of the whole surface.** Sizing it to the
//!    layer's bounds would save memory, but it would also put an offset into
//!    every coordinate the layer's contents already carry (they are absolute) —
//!    and the composite would need to map between two spaces. Full size means the
//!    instances, the scissor rects, and the composite UVs are all in one
//!    coordinate system, and the price is memory rather than correctness.
//! 2. **Blur strength is quantised by pass count.** The chain reaches its radius
//!    by halving resolution, so the visual radius roughly doubles per level and a
//!    requested radius picks a level count ([`levels_for`]). Finer control would
//!    need a per-pass offset uniform; the visual difference behind translucent UI
//!    does not pay for it.
//!
//! Targets are **pooled by nesting depth**, so sibling layers reuse one texture
//! and only genuinely nested ones allocate a second: the common case (a sidebar
//! material) is one texture plus its blur scratch.

use silka_paint::{Rect, Size};

use crate::geometry::SurfaceGeometry;

/// The blur shader, embedded at compile time (§3.2: no runtime shader assembly).
const BLUR_WGSL: &str = include_str!("shaders/blur.wgsl");
/// The composite shader, likewise.
const COMPOSITE_WGSL: &str = include_str!("shaders/composite.wgsl");

/// How many halved scratch levels each layer target keeps for blurring.
///
/// Three levels reach a visual radius of roughly 16–32 physical pixels, which
/// covers every material in the HIG and every `backdrop-blur` a designer asks
/// for. A fourth would cost another eighth of a screen of memory per layer for a
/// difference nobody can point at.
const BLUR_LEVELS: usize = 3;

/// The uniform block `composite.wgsl` expects: viewport, bounds, opacity.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct CompositeUniform {
    viewport: [f32; 2],
    rect_min: [f32; 2],
    rect_max: [f32; 2],
    opacity: f32,
    reserved: f32,
}

/// One texture with its view.
///
/// The texture handle is kept alongside the view purely to own the resource for
/// as long as the slot lives; nothing ever reads it, which is what the `allow`
/// says out loud rather than hiding behind an underscore.
#[derive(Debug)]
struct LayerTexture {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// One layer target: its full-size texture, its blur scratch levels, and a bind
/// group per level for sampling it.
#[derive(Debug)]
struct LayerSlot {
    /// Level 0 is the layer's own texture; 1..=[`BLUR_LEVELS`] are halved
    /// scratch levels.
    levels: Vec<LayerTexture>,
    /// `binds[i]` samples `levels[i]` — the whole state a blur pass needs, since
    /// the texel size comes from `textureDimensions` in the shader.
    binds: Vec<wgpu::BindGroup>,
}

/// The pool of layer targets plus the pipelines that operate on them.
#[derive(Debug)]
pub(crate) struct LayerStack {
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    slots: Vec<LayerSlot>,
    sampler: wgpu::Sampler,
    texture_layout: wgpu::BindGroupLayout,
    blur_down: wgpu::RenderPipeline,
    blur_up: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    /// One uniform buffer per composite in a frame. Pooled and reused, because
    /// `queue.write_buffer` writes all land before the frame's commands run — a
    /// single shared buffer would give every composite the last one's values.
    composite_uniforms: Vec<wgpu::Buffer>,
    /// How many composites the current frame has used.
    used_uniforms: usize,
}

impl LayerStack {
    /// Build the pipelines. No texture is allocated until a frame actually
    /// contains a layer, so an application without one pays nothing.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("silka.blur.wgsl"),
            source: wgpu::ShaderSource::Wgsl(BLUR_WGSL.into()),
        });
        let composite_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("silka.composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("silka.layer.sampler"),
            // Clamp: the blur's outer taps must not wrap around to the opposite
            // edge of the layer.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            // Linear is not optional here: the dual-Kawase filter relies on
            // bilinear taps landing between texels.
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silka.layer.texture.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silka.composite.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("silka.blur.pipeline.layout"),
            bind_group_layouts: &[Some(&texture_layout)],
            immediate_size: 0,
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("silka.composite.pipeline.layout"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });

        let blur_down = buat_pipeline_layar(
            device,
            "silka.blur.down",
            &blur_pipeline_layout,
            &blur_module,
            "fs_down",
            format,
            None,
        );
        let blur_up = buat_pipeline_layar(
            device,
            "silka.blur.up",
            &blur_pipeline_layout,
            &blur_module,
            "fs_up",
            format,
            None,
        );
        let composite = buat_pipeline_layar(
            device,
            "silka.composite",
            &composite_pipeline_layout,
            &composite_module,
            "fs_main",
            format,
            // The layer's contents are premultiplied, so this is the same
            // blending every other draw in this backend uses.
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        );

        Self {
            format,
            width: 0,
            height: 0,
            slots: Vec::new(),
            sampler,
            texture_layout,
            blur_down,
            blur_up,
            composite_layout,
            composite,
            composite_uniforms: Vec::new(),
            used_uniforms: 0,
        }
    }

    /// Make sure `slots_needed` targets of the current surface size exist.
    ///
    /// A resize drops every target: a layer texture is only ever valid for one
    /// surface size, and keeping a stale one would composite last frame's
    /// geometry at the wrong scale.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        geometry: SurfaceGeometry,
        slots_needed: usize,
    ) {
        let width = geometry.physical_width().max(1);
        let height = geometry.physical_height().max(1);
        if width != self.width || height != self.height {
            self.slots.clear();
            self.width = width;
            self.height = height;
        }
        // Also resets the per-frame uniform counter: `prepare` runs once per
        // frame, before any composite is recorded.
        self.used_uniforms = 0;
        while self.slots.len() < slots_needed {
            let slot = self.buat_slot(device);
            self.slots.push(slot);
        }
    }

    fn buat_slot(&self, device: &wgpu::Device) -> LayerSlot {
        let mut levels = Vec::with_capacity(BLUR_LEVELS + 1);
        for level in 0..=BLUR_LEVELS {
            let w = (self.width >> level).max(1);
            let h = (self.height >> level).max(1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("silka.layer.target"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            levels.push(LayerTexture { texture, view });
        }
        let binds = levels
            .iter()
            .map(|level| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("silka.layer.bind"),
                    layout: &self.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&level.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            })
            .collect();
        LayerSlot { levels, binds }
    }

    /// The view a layer's contents are drawn into.
    pub(crate) fn target_view(&self, slot: usize) -> Option<&wgpu::TextureView> {
        self.slots.get(slot).map(|s| &s.levels[0].view)
    }

    /// Blur a layer target in place, through the dual-Kawase chain.
    ///
    /// `radius` is in logical points; the physical radius (and therefore the pass
    /// count) follows the scale factor, so the same token looks the same on a 1×
    /// and a 2× display.
    pub(crate) fn blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        radius: f32,
        geometry: SurfaceGeometry,
    ) {
        let Some(target) = self.slots.get(slot) else {
            return;
        };
        let radius_px = radius * geometry.scale_factor() as f32;
        let levels = levels_for(radius_px);
        if levels == 0 {
            return;
        }
        for i in 0..levels {
            self.pass_layar(
                encoder,
                "silka.blur.down",
                &self.blur_down,
                &target.binds[i],
                &target.levels[i + 1].view,
            );
        }
        for i in (0..levels).rev() {
            self.pass_layar(
                encoder,
                "silka.blur.up",
                &self.blur_up,
                &target.binds[i + 1],
                &target.levels[i].view,
            );
        }
    }

    fn pass_layar(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every texel is written by the pass, so clearing is the
                    // cheapest load: it tells the driver not to fetch the old
                    // contents at all.
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..4, 0..1);
    }

    /// Composite a finished layer into its parent.
    ///
    /// `into` names the destination: `None` = the frame's final target (passed in
    /// as `final_target`, since this pool does not own it), `Some(i)` = the
    /// enclosing layer's texture. Records its own render pass with `LoadOp::Load`,
    /// because what is already on the destination is exactly what the layer has to
    /// sit on top of.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        final_target: &wgpu::TextureView,
        source: usize,
        into: Option<usize>,
        bounds: Rect,
        opacity: f32,
        viewport: Size,
    ) {
        let slot = source;
        if self.slots.len() <= slot || bounds.size.is_empty() || opacity <= 0.0 {
            return;
        }
        if into.is_some_and(|i| i >= self.slots.len()) {
            return;
        }
        let uniform = CompositeUniform {
            viewport: [viewport.width.max(1.0), viewport.height.max(1.0)],
            rect_min: [bounds.min_x(), bounds.min_y()],
            rect_max: [bounds.max_x(), bounds.max_y()],
            opacity: opacity.clamp(0.0, 1.0),
            reserved: 0.0,
        };

        let index = self.used_uniforms;
        if index >= self.composite_uniforms.len() {
            self.composite_uniforms
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("silka.composite.uniform"),
                    size: core::mem::size_of::<CompositeUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        self.used_uniforms += 1;
        let buffer = &self.composite_uniforms[index];

        // SAFETY: `CompositeUniform` is `repr(C)` holding only `f32` — no padding,
        // no pointers, no invalid bit patterns. The same reasoning as
        // `instance::as_bytes`.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&uniform as *const CompositeUniform) as *const u8,
                core::mem::size_of::<CompositeUniform>(),
            )
        };
        queue.write_buffer(buffer, 0, bytes);

        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("silka.composite.bind"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.slots[slot].levels[0].view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // Every remaining use of `self` is immutable, which is what lets the
        // destination view be borrowed out of the very pool this method mutated a
        // moment ago.
        let destination = match into {
            None => final_target,
            Some(i) => &self.slots[i].levels[0].view,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("silka.composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..4, 0..1);
    }
}

/// One screen-filling pipeline: four vertices, no vertex buffer.
fn buat_pipeline_layar(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    fragment_entry: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// How many down/up levels a blur radius asks for.
///
/// Each level halves the resolution, so the visual radius roughly doubles per
/// level. Below one pixel there is nothing to blur; above the ceiling the chain
/// would sample a texture smaller than the blur it is meant to produce.
fn levels_for(radius_px: f32) -> usize {
    if !radius_px.is_finite() || radius_px <= 1.0 {
        return 0;
    }
    (radius_px.log2().floor().max(1.0) as usize).min(BLUR_LEVELS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_composite_kelipatan_enam_belas_byte() {
        // A WebGPU uniform buffer alignment requirement.
        assert_eq!(core::mem::size_of::<CompositeUniform>() % 16, 0);
    }

    #[test]
    fn jumlah_level_naik_dengan_radius_lalu_dibatasi() {
        assert_eq!(levels_for(0.0), 0, "tanpa blur, tanpa pass");
        assert_eq!(levels_for(1.0), 0);
        assert_eq!(levels_for(2.0), 1);
        assert_eq!(levels_for(6.0), 2);
        assert_eq!(levels_for(12.0), 3);
        assert_eq!(levels_for(48.0), BLUR_LEVELS, "harus dibatasi");
        assert_eq!(levels_for(4096.0), BLUR_LEVELS);
    }

    #[test]
    fn radius_ngawur_tidak_membuat_pass() {
        for buruk in [f32::NAN, f32::INFINITY, -10.0] {
            assert_eq!(levels_for(buruk), 0, "radius {buruk}");
        }
    }

    #[test]
    fn shader_layar_ditanam_saat_kompilasi() {
        // The same "no runtime shaders" guard the SDF pipeline has: if either
        // source is ever read from disk, these constants are the first to go.
        assert!(BLUR_WGSL.contains("fn fs_down"));
        assert!(BLUR_WGSL.contains("fn fs_up"));
        assert!(COMPOSITE_WGSL.contains("fn fs_main"));
    }

    #[test]
    fn shader_layar_valid_dan_hanya_butuh_kapabilitas_downlevel() {
        for (nama, sumber, entri) in [
            ("blur", BLUR_WGSL, vec!["vs_main", "fs_down", "fs_up"]),
            ("composite", COMPOSITE_WGSL, vec!["vs_main", "fs_main"]),
        ] {
            let modul = naga::front::wgsl::parse_str(sumber)
                .unwrap_or_else(|e| panic!("{nama} gagal di-parse: {e:?}"));
            let mut validator = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            );
            validator
                .validate(&modul)
                .unwrap_or_else(|e| panic!("{nama} gagal divalidasi: {e:?}"));
            let ada: Vec<&str> = modul.entry_points.iter().map(|e| e.name.as_str()).collect();
            for e in entri {
                assert!(ada.contains(&e), "{nama} kehilangan {e}: {ada:?}");
            }
        }
    }
}
