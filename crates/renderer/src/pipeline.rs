//! Pipeline SDF: satu shader, satu draw call, semua kotak UI.
//!
//! **Semua varian dikompilasi di build time** (pelajaran Impeller, §3.2):
//! sumber WGSL masuk ke binary lewat [`include_str!`], modulnya dibuat sekali
//! saat device dibuat, dan perbedaan antar bentuk (arc vs squircle, ada/tidak
//! border, kotak vs bayangan) adalah **data instance** — bukan varian shader.
//! Tidak ada satu pun jalur kode yang merakit sumber shader saat runtime.

use rustui_paint::{GlyphSource, Size};

use crate::atlas::GlyphAtlasGpu;
use crate::geometry::SurfaceGeometry;
use crate::instance::{as_bytes, DrawList, QuadInstance};

/// Sumber shader, ditanam ke binary saat kompilasi Rust.
const SDF_WGSL: &str = include_str!("shaders/sdf.wgsl");

/// Isi uniform `Globals` di `sdf.wgsl`: viewport (poin logis) + padding.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Globals {
    viewport: [f32; 2],
    reserved: [f32; 2],
}

/// Jumlah instance awal yang dialokasikan; tumbuh dua kali lipat sesuai
/// kebutuhan dan tidak pernah menyusut, agar frame steady-state bebas alokasi.
const KAPASITAS_AWAL: usize = 256;

#[derive(Debug)]
pub(crate) struct SdfPipeline {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    kapasitas: usize,
    /// Atlas glyph di GPU — satu-satunya resource bertekstur pipeline ini.
    atlas: GlyphAtlasGpu,
    /// Revisi atlas yang sedang dirujuk `bind_group`.
    atlas_revision: u64,
}

impl SdfPipeline {
    /// Bangun pipeline untuk satu format target.
    ///
    /// Dipanggil sekali per surface; biayanya (kompilasi WGSL → MSL/SPIR-V oleh
    /// naga) dibayar di muka saat window dibuat, bukan saat frame pertama.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rustui.sdf.wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_WGSL.into()),
        });

        // Satu bind group untuk seluruh frame: uniform viewport + kedua atlas
        // glyph + sampler. Karena semuanya konstan sepanjang frame, tidak ada
        // pergantian bind group di tengah render pass — syarat agar seluruh
        // scene (kotak, bayangan, dan teks) muat dalam satu draw call.
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rustui.sdf.bind.layout"),
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
            ],
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rustui.sdf.globals"),
            size: core::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let atlas = GlyphAtlasGpu::new(device, format.is_srgb());
        let bind_group = buat_bind_group(device, &bind_group_layout, &globals, &atlas);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rustui.sdf.layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Lima vec4 berurutan — cerminan persis `QuadInstance`.
        let atribut = wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rustui.sdf"),
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
                // Empat titik per instance; tidak ada vertex buffer geometri.
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // UI tidak punya sisi belakang — culling hanya akan membuat
                // kotak menghilang saat matriks proyeksi berubah tanda.
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
                    // Shader menulis warna PREMULTIPLIED (lihat `premultiply`
                    // di sdf.wgsl) — inilah blending yang benar untuk tepi
                    // anti-alias dan bayangan bertumpuk.
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
        let atlas_revision = atlas.revision();

        Self {
            pipeline,
            globals,
            bind_group_layout,
            bind_group,
            instances,
            kapasitas: KAPASITAS_AWAL,
            atlas,
            atlas_revision,
        }
    }

    /// Unggah viewport, atlas glyph, dan instance frame ini.
    ///
    /// Atlas disinkronkan **sebelum** draw dan hanya sebesar kotak yang
    /// berubah; bind group dirakit ulang hanya kalau teksturnya benar-benar
    /// diganti (atlas tumbuh).
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: Size,
        list: &DrawList,
        glyphs: &mut dyn GlyphSource,
    ) {
        let instances = list.instances();
        self.atlas.sync(device, queue, glyphs);
        if self.atlas.revision() != self.atlas_revision {
            self.bind_group =
                buat_bind_group(device, &self.bind_group_layout, &self.globals, &self.atlas);
            self.atlas_revision = self.atlas.revision();
        }

        let globals = Globals {
            viewport: [viewport.width.max(1.0), viewport.height.max(1.0)],
            reserved: [0.0, 0.0],
        };
        // SAFETY: `Globals` adalah `repr(C)` berisi `f32` saja — sama alasannya
        // dengan `instance::as_bytes`.
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

    /// Rekam draw call untuk `list` — satu per batch clip.
    ///
    /// `list` harus yang sama dengan yang diberikan ke [`SdfPipeline::prepare`]
    /// pada frame ini.
    ///
    /// Scissor rect adalah **state render pass**, bukan parameter draw: ia
    /// hanya disetel ulang ketika kotaknya benar-benar berubah, sehingga scene
    /// tanpa clip tetap satu `draw` tanpa satu pun `set_scissor_rect`. Batch
    /// yang kotaknya menyusut jadi nol (viewport di luar surface, clip terbalik)
    /// **dilewati seluruhnya** — menggambarnya tanpa potong akan membocorkan
    /// isi scroll view ke seluruh window.
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        list: &DrawList,
        geometry: SurfaceGeometry,
    ) {
        // Keadaan awal render pass = seluruh attachment; ukurannya sama dengan
        // geometri surface, karena attachment-lah yang dikonfigurasi darinya.
        let Some(penuh) = geometry.full_scissor() else {
            return;
        };
        let mut terpasang = penuh;
        let mut siap = false;

        for batch in list.batches() {
            let scissor = match batch.clip {
                None => penuh,
                Some(rect) => match geometry.scissor(rect) {
                    Some(s) => s,
                    // Seluruhnya di luar surface: tidak ada piksel yang bisa
                    // lolos, jadi tidak ada gunanya merekam apa pun.
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
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rustui.sdf.bind"),
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
        ],
    })
}

fn buat_buffer_instance(device: &wgpu::Device, kapasitas: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rustui.sdf.instances"),
        size: (kapasitas * QuadInstance::SIZE) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Kapasitas berikutnya: berlipat ganda sampai cukup, supaya frame yang
/// tumbuh perlahan tidak mengalokasi buffer setiap frame.
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
        // Syarat alignment uniform buffer WebGPU.
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
        // Bukan sekadar uji sepele: ini yang menjaga janji "tanpa shader
        // runtime". Kalau suatu saat sumber shader dibaca dari file/dirakit
        // dari string, konstanta ini yang harus dihapus lebih dulu.
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
        // Validasi tanpa satu pun `Capabilities` di atas sudah membuktikan
        // shader tidak memakai fitur opsional — syarat agar jalur GL/Linux
        // lama tetap terbuka (§3.2).
        let modul = naga::front::wgsl::parse_str(SDF_WGSL).unwrap();
        assert_eq!(modul.entry_points.len(), 2, "hanya vs_main + fs_main");
    }
}
