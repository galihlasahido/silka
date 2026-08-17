//! The SDF pipeline: one shader, one draw call, every UI box.
//!
//! **Every variant is compiled at build time** (the Impeller lesson, §3.2): the
//! WGSL source goes into the binary via [`include_str!`], the module is created
//! once when the device is created, and the differences between shapes (arc vs
//! squircle, border or not, box vs shadow) are **instance data** — not shader
//! variants. Not a single code path assembles shader source at runtime.

use silka_paint::{GlyphSource, ImageSource, Size};

use crate::atlas::GlyphAtlasGpu;
use crate::geometry::SurfaceGeometry;
use crate::images::ImageAtlasGpu;
use crate::instance::{as_bytes, DrawList, QuadInstance};

/// The shader source, embedded into the binary when Rust is compiled.
const SDF_WGSL: &str = include_str!("shaders/sdf.wgsl");

/// The contents of the `Globals` uniform in `sdf.wgsl`: viewport (logical
/// points) + padding.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Globals {
    viewport: [f32; 2],
    reserved: [f32; 2],
}

/// The initial instance count allocated; it doubles as needed and never shrinks,
/// so the steady-state frame is allocation-free.
const KAPASITAS_AWAL: usize = 256;

#[derive(Debug)]
pub(crate) struct SdfPipeline {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    kapasitas: usize,
    /// The glyph atlas on the GPU.
    atlas: GlyphAtlasGpu,
    /// The image atlas on the GPU — photos, avatars, and monochrome icons.
    images: ImageAtlasGpu,
    /// The atlas revisions `bind_group` currently refers to, glyph and image.
    ///
    /// Both are needed: either atlas can grow independently, and a bind group
    /// pointing at a replaced texture would make text or images vanish without an
    /// error.
    atlas_revision: (u64, u64),
}

impl SdfPipeline {
    /// Build the pipeline for one target format.
    ///
    /// Called once per surface; its cost (naga compiling WGSL → MSL/SPIR-V) is
    /// paid up front when the window is created, not on the first frame.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("silka.sdf.wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_WGSL.into()),
        });

        // One bind group for the whole frame: the viewport uniform + both glyph
        // atlases + the sampler. Since all of it is constant for the frame,
        // there is no bind group switch mid render pass — a requirement for the
        // whole scene (boxes, shadows, and text) to fit in a single draw call.
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silka.sdf.bind.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // The image atlas rides in the same bind group as the glyph
                // atlases, which is what keeps an icon beside a label inside the
                // same single draw call.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("silka.sdf.globals"),
            size: core::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let atlas = GlyphAtlasGpu::new(device, format.is_srgb());
        let images = ImageAtlasGpu::new(device, format.is_srgb());
        let bind_group = buat_bind_group(device, &bind_group_layout, &globals, &atlas, &images);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("silka.sdf.layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Six consecutive vec4 — an exact mirror of `QuadInstance`, the last one
        // being the transform's linear part.
        let atribut = wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4,
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("silka.sdf"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: QuadInstance::SIZE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &atribut,
                }],
            },
            primitive: wgpu::PrimitiveState {
                // Four points per instance; no geometry vertex buffer.
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // A UI has no back faces — culling would only make boxes
                // disappear when the projection matrix flips sign.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // The shader writes PREMULTIPLIED color (see `premultiply`
                    // in sdf.wgsl) — this is the correct blending for
                    // anti-aliased edges and stacked shadows.
                    blend: Some(wgpu::BlendState {
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
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instances = buat_buffer_instance(device, KAPASITAS_AWAL);
        let atlas_revision = (atlas.revision(), images.revision());

        Self {
            pipeline,
            globals,
            bind_group_layout,
            bind_group,
            instances,
            kapasitas: KAPASITAS_AWAL,
            atlas,
            images,
            atlas_revision,
        }
    }

    /// Upload the viewport, the glyph atlas, and this frame's instances.
    ///
    /// The atlas is synced **before** drawing and only for the rect that
    /// changed; the bind group is rebuilt only when the texture itself was
    /// actually replaced (the atlas grew).
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: Size,
        list: &DrawList,
        glyphs: &mut dyn GlyphSource,
        images: &mut dyn ImageSource,
    ) {
        let instances = list.instances();
        self.atlas.sync(device, queue, glyphs);
        self.images.sync(device, queue, images);
        let revisi = (self.atlas.revision(), self.images.revision());
        if revisi != self.atlas_revision {
            self.bind_group = buat_bind_group(
                device,
                &self.bind_group_layout,
                &self.globals,
                &self.atlas,
                &self.images,
            );
            self.atlas_revision = revisi;
        }

        let globals = Globals {
            viewport: [viewport.width.max(1.0), viewport.height.max(1.0)],
            reserved: [0.0, 0.0],
        };
        // SAFETY: `Globals` is `repr(C)` holding only `f32` — the same reason as
        // in `instance::as_bytes`.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&globals as *const Globals) as *const u8,
                core::mem::size_of::<Globals>(),
            )
        };
        queue.write_buffer(&self.globals, 0, bytes);

        if instances.is_empty() {
            return;
        }
        if instances.len() > self.kapasitas {
            self.kapasitas = tumbuhkan(self.kapasitas, instances.len());
            self.instances = buat_buffer_instance(device, self.kapasitas);
        }
        queue.write_buffer(&self.instances, 0, as_bytes(instances));
    }

    /// Record the draw calls for one **range** of `list`'s clip batches — one
    /// `draw` per batch.
    ///
    /// `list` must be the same one handed to [`SdfPipeline::prepare`] for this
    /// frame.
    ///
    /// The range is what layers need: a frame containing one becomes several
    /// render passes over different targets, and each of them draws only the
    /// batches that belong to it. A frame without layers passes its whole range
    /// and does exactly the work it did before layers existed.
    ///
    /// The scissor rect is **render pass state**, not a draw parameter: it is
    /// only reset when the rect genuinely changes, so a scene without clipping
    /// stays one `draw` with not a single `set_scissor_rect`. A batch whose rect
    /// collapses to zero (a viewport outside the surface, an inverted clip) is
    /// **skipped entirely** — drawing it unclipped would spill a scroll view's
    /// contents across the whole window.
    pub(crate) fn draw_batches(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        list: &DrawList,
        geometry: SurfaceGeometry,
        first_batch: u32,
        last_batch: u32,
    ) {
        // A render pass starts out covering the whole attachment; its size
        // matches the surface geometry, since the attachment is configured
        // from it.
        let Some(penuh) = geometry.full_scissor() else {
            return;
        };
        let mut terpasang = penuh;
        let mut siap = false;

        let batches = list.batches();
        let mulai = (first_batch as usize).min(batches.len());
        let selesai = (last_batch as usize).clamp(mulai, batches.len());
        for batch in &batches[mulai..selesai] {
            let scissor = match batch.clip {
                None => penuh,
                Some(rect) => match geometry.scissor(rect) {
                    Some(s) => s,
                    // Entirely outside the surface: no pixel can get through,
                    // so there is no point recording anything.
                    None => continue,
                },
            };
            if batch.end <= batch.start {
                continue;
            }
            if !siap {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                siap = true;
            }
            if scissor != terpasang {
                pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
                terpasang = scissor;
            }
            pass.draw(0..4, batch.start..batch.end);
        }
    }
}

fn buat_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    atlas: &GlyphAtlasGpu,
    images: &ImageAtlasGpu,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("silka.sdf.bind"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(atlas.mask_view()),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(atlas.color_view()),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(atlas.sampler()),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(images.view()),
            },
        ],
    })
}

fn buat_buffer_instance(device: &wgpu::Device, kapasitas: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("silka.sdf.instances"),
        size: (kapasitas * QuadInstance::SIZE) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The next capacity: double until it is enough, so a frame that grows slowly
/// does not allocate a buffer every frame.
fn tumbuhkan(kapasitas: usize, dibutuhkan: usize) -> usize {
    let mut baru = kapasitas.max(1);
    while baru < dibutuhkan {
        baru *= 2;
    }
    baru
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_globals_kelipatan_enam_belas_byte() {
        // A WebGPU uniform buffer alignment requirement.
        assert_eq!(core::mem::size_of::<Globals>() % 16, 0);
    }

    #[test]
    fn kapasitas_berlipat_ganda_sampai_cukup() {
        assert_eq!(tumbuhkan(256, 100), 256);
        assert_eq!(tumbuhkan(256, 257), 512);
        assert_eq!(tumbuhkan(256, 5000), 8192);
        assert_eq!(tumbuhkan(0, 3), 4);
    }

    #[test]
    fn sumber_shader_ditanam_saat_kompilasi() {
        // Not a trivial test: this is what upholds the "no runtime shaders"
        // promise. If the shader source is ever read from a file or assembled
        // from strings, this constant is the first thing that has to go.
        assert!(SDF_WGSL.contains("fn vs_main"));
        assert!(SDF_WGSL.contains("fn fs_main"));
    }

    #[test]
    fn shader_wgsl_valid_dan_entry_pointnya_ada() {
        let modul = naga::front::wgsl::parse_str(SDF_WGSL).expect("WGSL gagal di-parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        let info = validator.validate(&modul).expect("WGSL gagal divalidasi");
        let _ = info;

        let entri: Vec<&str> = modul.entry_points.iter().map(|e| e.name.as_str()).collect();
        assert!(entri.contains(&"vs_main"), "entri: {entri:?}");
        assert!(entri.contains(&"fs_main"), "entri: {entri:?}");
    }

    #[test]
    fn shader_hanya_butuh_kapabilitas_downlevel() {
        // Validating with no `Capabilities` at all above already proves the
        // shader uses no optional features — the requirement for keeping the
        // older GL/Linux path open (§3.2).
        let modul = naga::front::wgsl::parse_str(SDF_WGSL).unwrap();
        assert_eq!(modul.entry_points.len(), 2, "hanya vs_main + fs_main");
    }
}
