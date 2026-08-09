//! # silka-paint
//!
//! Abstraksi perintah gambar tipis untuk seluruh framework (REKOMENDASI §3.2).
//!
//! Crate ini mendefinisikan *vocabulary* menggambar UI — bukan cara
//! mengeksekusinya di GPU:
//!
//! - **Rounded rect / squircle** — geometri sudut (continuous corner ala Apple)
//!   adalah *parameter* perintah gambar yang diteruskan ke shader, bukan
//!   konstanta (REKOMENDASI §2.7, §3.6). Preset Cupertino mengirim squircle,
//!   preset Tailwind mengirim arc biasa. Lihat [`CornerStyle`].
//! - **Glyph** — dirujuk lewat id atlas milik `silka-text`.
//! - **Shadow ganda** (ambient + key ala HIG) dan **blur** (dual-Kawase untuk
//!   materials) — butuh dukungan layer/offscreen texture di render graph.
//!
//! ## Kontrak yang MENGIKAT
//!
//! API publik crate ini **tidak boleh memuat tipe wgpu** (atau API grafis
//! lain). Kode widget hanya berbicara dalam perintah gambar crate ini;
//! `silka-renderer` (wgpu) adalah salah satu implementasi. Dengan begitu
//! backend baru (GL/CPU/BSD) bisa ditambah nanti di satu tempat tanpa
//! menulis ulang framework (REKOMENDASI §5 failure mode #7).
//!
//! ## Status
//!
//! Kosakata yang sudah ada: warna (dengan konversi ruang sRGB→linear yang
//! benar), geometri poin-logis, geometri sudut sebagai parameter, dan
//! [`Scene`] berisi daftar [`Command`]. Rasterisasi `Command` sendiri masuk
//! milestone shader SDF; backend hari ini baru mengeksekusi warna latar
//! [`Scene::clear_color`].
//!
//! ```
//! use silka_paint::{Color, Corners, CornerStyle, Quad, Rect, Scene};
//!
//! let mut scene = Scene::new(Color::hex(0x1C1C1E));
//! scene.push(
//!     Quad::new(Rect::new(24.0, 24.0, 180.0, 96.0))
//!         .background(Color::hex(0x2C2C2E))
//!         // Bentuk sudut datang dari token theme, bukan dari literal di sini.
//!         .corners(Corners::uniform(14.0, CornerStyle::squircle()))
//!         .normalized(),
//! );
//! assert_eq!(scene.len(), 1);
//! ```

#![warn(missing_docs)]

pub mod atlas;
pub mod color;
pub mod corner;
pub mod geometry;
pub mod glyph;
pub mod scene;
pub mod shadow;

pub use atlas::{AtlasRegion, GlyphFormat, GlyphPlacement, GlyphSource, NoGlyphs};
pub use color::{linear_to_srgb, srgb_to_linear, Color};
pub use corner::{CornerRadii, CornerStyle, Corners};
pub use geometry::{Insets, Point, Rect, Size};
pub use glyph::{Glyph, GlyphImageId, GlyphRun};
pub use scene::{Command, Quad, Scene, ShadowQuad};
pub use shadow::{Shadow, ShadowPair};
