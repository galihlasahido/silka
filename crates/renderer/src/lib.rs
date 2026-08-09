//! # rustui-renderer
//!
//! Backend **wgpu** — satu-satunya tempat di workspace yang boleh menyentuh
//! tipe wgpu (REKOMENDASI §3.2). Mengimplementasikan perintah gambar
//! `rustui-paint` dengan shader SDF khusus UI ala GPUI:
//!
//! - Rounded rect + **squircle** (superellipse G2-continuous) langsung di SDF
//!   shader; radius/kelengkungan datang sebagai parameter per-perintah.
//! - Shadow ganda ambient + key, border, glyph dari atlas, ikon monochrome.
//! - Blur dalam-aplikasi (dual-Kawase) lewat layer/offscreen texture.
//!
//! Pelajaran yang MENGIKAT dari Impeller: **semua varian shader dikompilasi
//! di build time** — tidak pernah generate shader saat runtime (§3.2).
//! Render hanya saat dirty; vsync lewat display link per platform (§3.5).
//!
//! Backend alternatif di masa depan (vello_hybrid GL, tiny-skia CPU) menjadi
//! crate saudara yang mengimplementasikan `rustui-paint` yang sama.
//!
//! ## Yang sudah ada (milestone `window-wgpu` + `sdf-shader` + `glyph-gpu-bridge` + `clip-gpu`)
//!
//! Fondasi surface: [`Gpu`] (instance/adapter/device/queue, Metal di macOS),
//! [`WindowSurface`] (swapchain, resize, DPI), dan konversi ruang warna
//! sRGB→linear yang benar.
//!
//! Di atasnya, pipeline SDF (`shaders/sdf.wgsl`) merasterisasi seluruh
//! kosakata kotak dalam **satu draw call**:
//!
//! | Yang berbeda | Bagaimana dinyatakan |
//! |---|---|
//! | Arc vs squircle | eksponen superellipse per instance (2 vs ≈4) |
//! | Radius per sudut | empat `f32` per instance, sudah diskalakan CPU-side |
//! | Border | tebal per instance; cincin antara dua isoline SDF |
//! | Shadow ambient + key | dua instance ber-blur gaussian di belakang kotak |
//! | **Glyph** | instance bertekstur: UV atlas + warna run dari token theme |
//!
//! Karena semuanya data, **tidak ada varian shader** dan tidak ada WGSL yang
//! dirakit saat runtime. Anti-alias diturunkan dari derivatif layar sehingga
//! benar di Retina 2× maupun scale pecahan Wayland tanpa parameter tambahan.
//!
//! ### Teks
//!
//! Perintah `GlyphRun` menjadi quad bertekstur yang men-sample glyph atlas.
//! Yang menjaga teks tetap tajam dan murah:
//!
//! - **Kotak tujuan disetel ke grid piksel fisik** sehingga satu texel jatuh
//!   tepat pada satu piksel layar (tajam di 2×); subpixel *positioning* tetap
//!   utuh karena ia terkandung di dalam bitmap yang dipilih lapisan teks.
//! - **Unggah inkremental**: hanya kotak atlas yang berubah yang dikirim ke
//!   GPU — nol byte pada frame yang teksnya tidak berubah.
//! - **Satu draw call untuk seluruh scene**: teks ikut dalam urutan perintah
//!   yang sama dengan kotak dan bayangan, jadi teks selalu di atas latarnya.
//! - Atlasnya datang dari [`rustui_paint::GlyphSource`] — backend tidak pernah
//!   menyebut `rustui-text`, dan `rustui-text` tidak pernah menyebut wgpu.
//!
//! ### Clip
//!
//! `Command::PushClip`/`PopClip` menjadi **scissor rect GPU**: scene dipecah
//! menjadi daftar batch `(kotak potong, rentang instance)` yang urutannya
//! persis urutan perintah, dan batch baru hanya dibuka saat kotak potongnya
//! berubah — UI tanpa clip tetap satu draw call, satu scroll view menambah dua.
//! Kotaknya dipakai apa adanya karena irisan clip bersarang sudah diselesaikan
//! `rustui-core`; yang tetap dipelihara backend hanyalah ingatan akan kotak
//! induk untuk dipulihkan saat `PopClip`. Konversi poin logis → piksel fisik
//! lewat [`SurfaceGeometry`] membulatkan **ke luar** (tepi konten tidak pernah
//! termakan) dan menjepit ke batas surface (scissor di luar batas = validation
//! error wgpu). Batch yang kotaknya kosong dilewati seluruhnya.
//!
//! Jalur yang sama tersedia tanpa window lewat [`Gpu::headless`] +
//! [`OffscreenTarget`] — fondasi golden/snapshot test visual di CI (§9.5),
//! termasuk uji "piksel teks benar-benar ada" di `tests/teks.rs`.
//!
//! ## Batas yang dijaga
//!
//! Permukaan publik crate ini hanya memakai tipe `rustui-paint` dan
//! `raw-window-handle`. Ia **tidak** tahu apa itu winit, dan pemanggilnya
//! **tidak** perlu tahu apa itu wgpu. Satu-satunya pintu ke dunia wgpu adalah
//! [`Gpu::device`], yang khusus untuk crate backend saudara.
//!
//! ```no_run
//! use std::sync::Arc;
//! use rustui_paint::{Color, Scene, Size};
//! use rustui_renderer::{Gpu, SurfaceGeometry};
//!
//! # fn contoh<W: rustui_renderer::WindowTarget>(window: Arc<W>) -> Result<(), Box<dyn std::error::Error>> {
//! let geometry = SurfaceGeometry::from_logical(Size::new(1024.0, 720.0), 2.0);
//! let (gpu, mut surface) = Gpu::with_surface(window, geometry)?;
//!
//! // Warna latar selalu datang dari token theme, tidak pernah literal.
//! let scene = Scene::new(Color::hex(0x1C1C1E));
//! surface.render(&gpu, &scene)?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod atlas;
mod error;
mod format;
mod geometry;
mod gpu;
mod instance;
mod offscreen;
mod pipeline;
mod surface;

pub use error::RendererError;
pub use geometry::SurfaceGeometry;
pub use gpu::{Gpu, WindowTarget};
pub use offscreen::{OffscreenTarget, Rgba8Image};
pub use surface::{FrameOutcome, WindowSurface};
