//! # rustui-text
//!
//! Wrapper tipis di atas **cosmic-text** (REKOMENDASI §3.3) — lapisan tersulit
//! di seluruh framework dan pembunuh #1 framework GUI baru (§5 failure mode #1).
//! Kontrak yang MENGIKAT: **jangan pernah menulis shaper sendiri.**
//!
//! Tanggung jawab crate ini:
//!
//! - **Shaping** lewat cosmic-text (fontdb + rustybuzz + swash), termasuk font
//!   fallback per platform, bidi (UAX #9), dan emoji ZWJ/warna.
//! - **Glyph atlas** — glyph di-rasterisasi ke atlas dan dirujuk lewat id;
//!   `rustui-paint` hanya berbicara dalam id itu, tidak pernah menyentuh font.
//! - **Measure** — fungsi ukur untuk leaf node layout, dipakai `rustui-core`
//!   (box constraints) dan Taffy lewat measure function (§3.4).
//!
//! Yang wajib benar sejak awal (§3.3): subpixel *positioning* (bukan subpixel
//! AA), gerakan kursor per grapheme cluster (UAX #29), dan preedit IME yang
//! dirender inline. `parley` adalah arah masa depan, tapi bukan untuk v1.
//!
//! ## Batas yang dijaga
//!
//! Tipe cosmic-text **tidak pernah muncul di API publik** crate ini. Pemanggil
//! berbicara dalam [`TextStyle`], [`TextConstraints`], [`TextMeasure`],
//! [`TextLayout`], dan `rustui_paint::GlyphRun`. Dengan begitu pindah ke
//! `parley` nanti adalah pekerjaan di dalam crate ini saja, dan kode widget
//! tetap tidak tahu apa itu font (§3.2, §3.3).
//!
//! ## Alur pemakaian
//!
//! ```
//! use rustui_paint::{Color, Point, Scene};
//! use rustui_text::{TextConstraints, TextEngine, TextStyle};
//!
//! // Satu mesin untuk seluruh aplikasi. `bundled_only` = tanpa font sistem,
//! // dipakai test/CI agar hasilnya deterministik.
//! let mut teks = TextEngine::bundled_only();
//! teks.set_scale_factor(2.0); // Retina
//!
//! // Gaya selalu dibangun dari token theme, tidak pernah angka literal.
//! let gaya = TextStyle::new().size(17.0);
//!
//! // 1. Ukur — inilah yang dipakai sistem layout (box constraints, §3.4).
//! let ukuran = teks.measure("Halo, dunia", &gaya, TextConstraints::width(280.0));
//! assert!(ukuran.width() > 0.0 && ukuran.line_count == 1);
//!
//! // 2. Gambar — hasilnya perintah `GlyphRun` berisi id atlas, bukan font.
//! let mut scene = Scene::new(Color::hex(0x1C1C1E));
//! teks.draw(
//!     &mut scene,
//!     "Halo, dunia",
//!     &gaya,
//!     TextConstraints::width(280.0),
//!     Point::new(24.0, 24.0),
//!     Color::WHITE,
//! );
//! assert_eq!(scene.len(), 1);
//! ```
//!
//! ## Yang dilihat backend
//!
//! Perintah `GlyphRun` hanya membawa **id atlas + kotak tujuan logis**. Backend
//! menukar id itu jadi tekstur lewat trait [`rustui_paint::GlyphSource`], yang
//! diimplementasikan [`TextEngine`] (dan [`GlyphCache`]) — itulah satu-satunya
//! permukaan yang dilihat backend:
//!
//! ```
//! use rustui_paint::{GlyphFormat, GlyphSource};
//! use rustui_text::TextEngine;
//!
//! let mut teks = TextEngine::bundled_only();
//! # let gaya = rustui_text::TextStyle::new();
//! # let l = teks.layout("Halo", &gaya, rustui_text::TextConstraints::UNBOUNDED);
//! # let run = teks.rasterize(&l, rustui_paint::Point::ZERO, rustui_paint::Color::WHITE);
//!
//! // Yang dilakukan backend tiap frame — tanpa pernah menyebut "font":
//! let sisi = teks.atlas_size(GlyphFormat::Mask);
//! if let Some(kotak) = teks.take_dirty(GlyphFormat::Mask) {
//!     let _piksel = teks.atlas_pixels(GlyphFormat::Mask); // unggah kotak ini saja
//!     let _uv = kotak.uv(sisi);
//! }
//! let _letak = teks.placement(run.glyphs[0].image); // id → kotak di atlas
//! ```
//!
//! Tidak ada tipe GPU di sini: backend wgpu hari ini dan backend GL/CPU nanti
//! membaca sumber yang sama (§3.2, §5 failure mode #7). Sebaliknya, backend
//! juga tidak perlu tahu crate ini ada — ia hanya memegang `&mut dyn
//! GlyphSource`, sehingga pindah ke `parley` nanti tidak menyentuh renderer.
//!
//! ## Yang belum ada (utang teknis yang disadari)
//!
//! - Axis `opsz` (optical size) Inter belum di-set otomatis per ukuran font;
//!   yang sudah jalan adalah axis `wght` lewat berat variable font (§3.6).
//! - Rich text (banyak gaya dalam satu paragraf) dan ellipsis otomatis; yang
//!   tersedia baru `max_lines` + penanda `overflowed` sebagai fondasinya.
//! - Rentang seleksi yang menyeberangi baris belum menyorot pemisah barisnya
//!   sendiri (terlihat saat `text_area` multi-baris nanti); yang sudah benar
//!   adalah sorotan per potongan visual di dalam tiap baris.
//!
//! Yang **sudah** ada dan dulu tercatat sebagai utang: editing, caret per
//! grapheme, dan preedit IME hidup di [`edit`] (model murni, tanpa piksel),
//! dengan geometrinya — [`TextLayout::hit`], [`TextLayout::caret`], dan
//! [`TextLayout::selection_rects`] — di [`layout`]. Keduanya dipakai
//! `text_field` (KOMPONEN.md Tier 2).

#![warn(missing_docs)]

pub mod atlas;
pub mod cache;
pub mod edit;
pub mod engine;
pub mod font;
pub mod layout;
pub mod measure;
pub mod style;

pub use atlas::{AtlasFormat, AtlasRect, GlyphAtlas};
pub use edit::{Movement, Preedit, Selection, TextEdit};
pub use cache::{FontId, GlyphCache, GlyphImage, GlyphKey, GlyphLookup, RasterGlyph, SubpixelBin};
pub use engine::TextEngine;
pub use font::{FontOptions, BUNDLED_UI_FONT};
pub use layout::{Caret, LineMetrics, TextLayout};
pub use measure::{TextConstraints, TextMeasure};
pub use style::{FontFamily, FontWeight, TextAlign, TextStyle, TextWrap};
