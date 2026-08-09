//! Pengepakan [`Scene`] menjadi data instance untuk shader SDF.
//!
//! Semua keputusan geometri dan warna terjadi **di sini** — CPU, murni nilai,
//! bisa diuji tanpa GPU sama sekali. Shader hanya mengeksekusi apa yang sudah
//! diputuskan modul ini:
//!
//! - radius per sudut sudah dikalikan faktor squircle dan dibatasi terhadap
//!   ukuran kotak (§3.6: geometri sudut adalah parameter, bukan konstanta);
//! - eksponen superellipse sudah diturunkan dari [`silka_paint::CornerStyle`];
//! - warna sudah dipindahkan ke ruang yang benar untuk format target.
//!
//! Konsekuensinya "shader menggambar squircle dengan benar" bisa diregresi-uji
//! di CI tanpa GPU, dan satu-satunya yang tersisa untuk diuji secara visual
//! adalah rasterisasinya sendiri.

use silka_paint::{
    Color, Command, Corners, GlyphFormat, GlyphRun, GlyphSource, Quad, Rect, Scene, ShadowQuad,
    Size,
};

/// Jenis instance di `params.w` — harus sama dengan konstanta di `sdf.wgsl`.
const KIND_QUAD: f32 = 0.0;
const KIND_SHADOW: f32 = 1.0;
const KIND_GLYPH: f32 = 2.0;

/// Pemilih atlas di `params.x` untuk instance glyph — cerminan `sdf.wgsl`.
const ATLAS_MASK: f32 = 0.0;
const ATLAS_COLOR: f32 = 1.0;

/// Satu instance untuk shader SDF.
///
/// Tata letaknya adalah kontrak dengan `sdf.wgsl`: lima `vec4<f32>` berurutan,
/// tanpa padding tersembunyi (semua field `f32`, `repr(C)`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct QuadInstance {
    /// xy = pusat, zw = setengah ukuran, poin logis.
    pub bounds: [f32; 4],
    /// Radius kiri-atas, kanan-atas, kanan-bawah, kiri-bawah — sudah final.
    ///
    /// Pada instance **glyph** slot yang sama membawa kotak UV
    /// `[u0, v0, u1, v1]`: bentuk sudut tidak berlaku untuk bitmap, jadi
    /// tidak ada gunanya menambah field yang selalu nol untuk semua kotak
    /// biasa (satu tata letak instance = satu pipeline = satu draw call).
    pub radii: [f32; 4],
    /// Warna isi (atau warna bayangan / warna teks), straight alpha.
    pub background: [f32; 4],
    /// Warna border, straight alpha. Tidak dipakai instance glyph.
    pub border: [f32; 4],
    /// x = tebal border (glyph: pemilih atlas), y = eksponen superellipse,
    /// z = sigma, w = jenis.
    pub params: [f32; 4],
}

impl QuadInstance {
    /// Ukuran satu instance dalam byte (= `array_stride` vertex buffer).
    pub const SIZE: usize = core::mem::size_of::<QuadInstance>();

    /// Benar bila instance ini benar-benar bisa menghasilkan piksel.
    ///
    /// Dipakai untuk membuang perintah tak terlihat sebelum menyentuh GPU:
    /// kotak berukuran nol, warna transparan penuh, border setebal nol.
    fn is_visible(&self) -> bool {
        let punya_luas = self.bounds[2] > 0.0 && self.bounds[3] > 0.0;
        let isi = self.background[3] > 0.0;
        let garis = self.params[0] > 0.0 && self.border[3] > 0.0;
        punya_luas && (isi || garis)
    }
}

/// Ruang warna yang diharapkan target render.
///
/// Format `*Srgb` melakukan encoding balik di hardware, jadi shader harus
/// menulis nilai **linear**. Ini titik konversi yang sama disiplinnya dengan
/// `format::clear_color` — kalau dilewatkan, seluruh UI tampak "cuci".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    /// Target `*Srgb`: kirim nilai linear.
    Linear,
    /// Target non-sRGB: kirim nilai apa adanya.
    Srgb,
}

impl ColorSpace {
    fn encode(self, color: Color) -> [f32; 4] {
        match self {
            ColorSpace::Linear => color.to_linear(),
            ColorSpace::Srgb => color.components(),
        }
    }
}

/// Rentang instance berurutan yang berbagi satu kotak potong.
///
/// Satu batch = satu `set_scissor_rect` + satu `draw`. Batch baru dibuat
/// **hanya** saat clip efektif berubah, sehingga scene tanpa clip tetap
/// menjadi satu draw call seperti sebelumnya, dan scroll view menambah tepat
/// dua batch (isi terpotong, lalu kembali ke luar), bukan satu per perintah.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct InstanceBatch {
    /// Kotak potong efektif dalam **poin logis absolut**; `None` = tanpa potong.
    pub clip: Option<Rect>,
    /// Indeks instance pertama.
    pub start: u32,
    /// Indeks setelah instance terakhir.
    pub end: u32,
}

/// Seluruh isi satu frame: instance urut belakang→depan, dipecah menjadi batch
/// per kotak potong.
///
/// Dipakai ulang antar frame (`clear` tidak melepas kapasitas) supaya frame
/// steady-state tetap bebas alokasi (§3.5).
#[derive(Debug, Default)]
pub(crate) struct DrawList {
    instances: Vec<QuadInstance>,
    batches: Vec<InstanceBatch>,
    /// Tumpukan clip — **hanya untuk memulihkan**, tidak pernah untuk mengiris.
    ///
    /// Irisan clip bersarang sudah diselesaikan `silka-core`: `PushClip`
    /// membawa kotak yang sudah diiriskan dengan clip di luarnya (lihat
    /// `child_clip` di `core::tree::paint`). Yang tetap dibutuhkan backend
    /// hanyalah **ingatan** akan kotak induk, karena `PopClip` berarti
    /// "kembali ke clip sebelumnya" dan kotak itu tidak dikirim ulang. Tanpa
    /// tumpukan, dua scroll view bersarang akan meneruskan clip yang lebih
    /// sempit ke saudara di luar viewport dalam.
    stack: Vec<Rect>,
}

impl DrawList {
    /// Semua instance frame ini, urut gambar.
    pub(crate) fn instances(&self) -> &[QuadInstance] {
        &self.instances
    }

    /// Batch frame ini, urut gambar.
    pub(crate) fn batches(&self) -> &[InstanceBatch] {
        &self.batches
    }

    /// Benar bila tidak ada satu instance pun untuk digambar.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    fn clear(&mut self) {
        self.instances.clear();
        self.batches.clear();
        self.stack.clear();
    }

    /// Kotak potong yang sedang berlaku.
    fn clip(&self) -> Option<Rect> {
        self.stack.last().copied()
    }

    fn push_clip(&mut self, rect: Rect) {
        self.stack.push(rect);
    }

    fn pop_clip(&mut self) {
        let ada = self.stack.pop().is_some();
        // `Scene` menjamin pasangannya seimbang; kalau tidak, lebih baik frame
        // tergambar tanpa potong daripada panik di tengah jalur render.
        debug_assert!(ada, "PopClip tanpa PushClip");
    }

    /// Tambahkan satu instance ke batch yang sedang terbuka, buka batch baru
    /// bila clip-nya berbeda.
    fn push(&mut self, instance: QuadInstance) {
        if !instance.is_visible() {
            return;
        }
        let clip = self.clip();
        // Clip degenerate (viewport yang menyusut jadi nol) tidak bisa
        // meloloskan satu piksel pun — instance-nya tidak perlu sampai ke GPU.
        if clip.is_some_and(|c| c.size.is_empty()) {
            return;
        }
        let index = self.instances.len() as u32;
        match self.batches.last_mut() {
            Some(batch) if batch.clip == clip => batch.end = index + 1,
            _ => self.batches.push(InstanceBatch {
                clip,
                start: index,
                end: index + 1,
            }),
        }
        self.instances.push(instance);
    }

    fn reserve(&mut self, tambahan: usize) {
        self.instances.reserve(tambahan);
    }
}

/// Ubah seluruh perintah sebuah scene menjadi instance, urut belakang→depan.
///
/// **Urutan adalah kontraknya**: instance dikeluarkan persis seurut perintah
/// scene, dan seluruhnya digambar dalam satu draw call oleh satu pipeline —
/// sehingga teks selalu berada di atas latar yang mendahuluinya, tidak pernah
/// tertimpa (blending mengikuti urutan primitif di dalam draw call).
///
/// `scale_factor` dipakai untuk **menyetel kotak tujuan glyph ke grid piksel
/// fisik**: bitmap glyph dirasterisasi pada resolusi layar (§3.3), jadi satu
/// texel harus jatuh tepat pada satu piksel layar. Tanpa penyetelan ini teks
/// di layar 2× akan lembek karena disampel di antara dua texel. Subpixel
/// *positioning* tidak ikut hilang: ia sudah terkandung di dalam bitmap yang
/// dipilih lapisan teks, bukan di posisi kotaknya.
///
/// [`Command::PushClip`]/[`Command::PopClip`] **tidak** menghasilkan instance:
/// keduanya memecah daftar menjadi [`InstanceBatch`] yang nanti dipasang
/// sebagai scissor rect GPU. Kotaknya dipakai apa adanya — irisan clip
/// bersarang sudah dilakukan `silka-core` sebelum perintahnya dibuat.
///
/// Perintah yang belum didukung backend ini dilewati **secara eksplisit**
/// (lihat lengan `match` di bawah) supaya "belum ada" tidak pernah tersamar
/// sebagai "sudah jalan" — `Command` sengaja `#[non_exhaustive]`.
pub(crate) fn fill_draw_list(
    scene: &Scene,
    space: ColorSpace,
    scale_factor: f32,
    glyphs: &dyn GlyphSource,
    out: &mut DrawList,
) {
    out.clear();
    out.reserve(scene.len());
    for command in scene.commands() {
        match command {
            Command::Quad(q) => out.push(quad_instance(q, space)),
            Command::Shadow(s) => out.push(shadow_instance(s, space)),
            Command::GlyphRun(r) => fill_glyph_run(r, space, scale_factor, glyphs, out),
            Command::PushClip(rect) => out.push_clip(*rect),
            Command::PopClip => out.pop_clip(),
            // Kosakata `silka-paint` masih tumbuh (blur/material, layer
            // offscreen). Perintah baru yang belum punya jalur di sini
            // dilewatkan agar frame tetap tergambar — tapi ia HARUS muncul
            // sebagai lengan bernama di atas begitu backend mendukungnya.
            lain => debug_assert!(false, "perintah gambar belum didukung backend: {lain:?}"),
        }
    }
}

/// Versi yang mengalokasi sendiri — dipakai test dan tooling headless.
#[cfg(test)]
pub(crate) fn draw_list_from_scene(
    scene: &Scene,
    space: ColorSpace,
    scale_factor: f32,
    glyphs: &dyn GlyphSource,
) -> DrawList {
    let mut out = DrawList::default();
    fill_draw_list(scene, space, scale_factor, glyphs, &mut out);
    out
}

/// Satu [`GlyphRun`] → satu instance quad bertekstur per glyph.
///
/// Semua glyph run ini memakai warna yang sama (kontrak `GlyphRun`), jadi
/// warnanya dikodekan sekali di sini, bukan per glyph.
fn fill_glyph_run(
    run: &GlyphRun,
    space: ColorSpace,
    scale_factor: f32,
    glyphs: &dyn GlyphSource,
    out: &mut DrawList,
) {
    if run.is_empty() || run.color.a <= 0.0 {
        return;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let warna = space.encode(run.color);
    out.reserve(run.len());

    for glyph in &run.glyphs {
        // Id yang sudah hangus (atlas dibangun ulang saat penuh) hilang untuk
        // satu frame — jauh lebih baik daripada menggambar glyph yang salah.
        let Some(letak) = glyphs.placement(glyph.image) else {
            continue;
        };
        if letak.region.is_empty() {
            continue;
        }
        let ukuran_atlas = glyphs.atlas_size(letak.format);
        if ukuran_atlas == 0 {
            continue;
        }

        // Kotak tujuan dalam piksel FISIK, disetel ke grid piksel: lebar dan
        // tingginya persis sebesar bitmap di atlas.
        let mut x0 = (glyph.bounds.min_x() * scale).round();
        let mut y0 = (glyph.bounds.min_y() * scale).round();
        let mut x1 = x0 + letak.region.width as f32;
        let mut y1 = y0 + letak.region.height as f32;

        let [mut u0, mut v0, mut u1, mut v1] = letak.region.uv(ukuran_atlas);

        // Clip run (truncation/ellipsis, scroll view) diselesaikan di CPU:
        // kotak dipotong dan UV ikut dipotong secara proporsional. Dengan
        // begitu tidak ada `discard` di shader dan tidak ada scissor rect per
        // glyph yang akan memecah batch.
        if let Some(clip) = run.clip {
            let (cx0, cy0) = (clip.min_x() * scale, clip.min_y() * scale);
            let (cx1, cy1) = (clip.max_x() * scale, clip.max_y() * scale);
            let nx0 = x0.max(cx0);
            let ny0 = y0.max(cy0);
            let nx1 = x1.min(cx1);
            let ny1 = y1.min(cy1);
            if nx1 <= nx0 || ny1 <= ny0 {
                continue;
            }
            let (lebar, tinggi) = (x1 - x0, y1 - y0);
            let (du, dv) = (u1 - u0, v1 - v0);
            let (au0, av0) = ((nx0 - x0) / lebar, (ny0 - y0) / tinggi);
            let (au1, av1) = ((nx1 - x0) / lebar, (ny1 - y0) / tinggi);
            u1 = u0 + du * au1;
            v1 = v0 + dv * av1;
            u0 += du * au0;
            v0 += dv * av0;
            x0 = nx0;
            y0 = ny0;
            x1 = nx1;
            y1 = ny1;
        }

        // Kembali ke poin logis: shader hanya mengenal satu ruang koordinat.
        let instance = QuadInstance {
            bounds: [
                (x0 + x1) * 0.5 / scale,
                (y0 + y1) * 0.5 / scale,
                (x1 - x0) * 0.5 / scale,
                (y1 - y0) * 0.5 / scale,
            ],
            radii: [u0, v0, u1, v1],
            background: warna,
            border: [0.0; 4],
            params: [
                match letak.format {
                    GlyphFormat::Mask => ATLAS_MASK,
                    GlyphFormat::Color => ATLAS_COLOR,
                },
                // Eksponen 2 = jalur `length()` di shader: instance glyph tidak
                // pernah memakai SDF, tapi nilainya tetap harus waras karena
                // `fwidth` dihitung sebelum percabangan jenis.
                2.0,
                0.0,
                KIND_GLYPH,
            ],
        };
        out.push(instance);
    }
}

fn quad_instance(quad: &Quad, space: ColorSpace) -> QuadInstance {
    let batas = (quad.rect.size.min_side() * 0.5).max(0.0);
    QuadInstance {
        bounds: bounds_of(quad.rect),
        radii: radii_of(quad.corners, quad.rect.size),
        background: space.encode(quad.background),
        border: space.encode(quad.border_color),
        params: [
            quad.border_width.clamp(0.0, batas),
            quad.corners.style.superellipse_exponent(),
            0.0,
            KIND_QUAD,
        ],
    }
}

fn shadow_instance(shadow: &ShadowQuad, space: ColorSpace) -> QuadInstance {
    QuadInstance {
        bounds: bounds_of(shadow.rect),
        radii: radii_of(shadow.corners, shadow.rect.size),
        background: space.encode(shadow.color),
        border: [0.0; 4],
        params: [
            0.0,
            shadow.corners.style.superellipse_exponent(),
            shadow.sigma().max(0.0),
            KIND_SHADOW,
        ],
    }
}

fn bounds_of(rect: Rect) -> [f32; 4] {
    let c = rect.center();
    [
        c.x,
        c.y,
        (rect.size.width * 0.5).max(0.0),
        (rect.size.height * 0.5).max(0.0),
    ]
}

/// Radius final per sudut: radius nominal × faktor squircle, dibatasi ke
/// separuh sisi terpendek.
///
/// Faktor inilah yang membuat sudut Apple "mulai melengkung lebih awal"
/// (≈1.528× radius nominal, §3.6). Karena ia dikalikan di sini, shader hanya
/// menerima angka jadi dan tidak perlu tahu apa itu preset.
fn radii_of(corners: Corners, size: Size) -> [f32; 4] {
    let batas = (size.min_side() * 0.5).max(0.0);
    let faktor = corners.style.extent_factor();
    let skala = |r: f32| (r.max(0.0) * faktor).min(batas);
    [
        skala(corners.radii.top_left),
        skala(corners.radii.top_right),
        skala(corners.radii.bottom_right),
        skala(corners.radii.bottom_left),
    ]
}

/// Pandang slice instance sebagai byte mentah untuk diunggah ke GPU.
///
/// `unsafe` di sini disengaja dan terkurung: [`QuadInstance`] adalah `repr(C)`
/// berisi `f32` saja — tanpa padding, tanpa pointer, tanpa bit pattern tak
/// valid — sehingga setiap byte-nya bisa dibaca. Ini satu-satunya alternatif
/// selain menambah dependensi hanya untuk sebuah cast (REKOMENDASI §4:
/// minimalkan `unsafe`, dan konsentrasikan di batas GPU).
pub(crate) fn as_bytes(instances: &[QuadInstance]) -> &[u8] {
    // SAFETY: lihat dokumentasi di atas; panjangnya persis n * SIZE dan
    // umurnya terikat ke slice asal.
    unsafe {
        core::slice::from_raw_parts(
            instances.as_ptr() as *const u8,
            core::mem::size_of_val(instances),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_paint::{
        AtlasRegion, CornerStyle, Glyph, GlyphImageId, GlyphPlacement, NoGlyphs, Shadow, ShadowPair,
    };
    use std::collections::HashMap;

    /// Atlas palsu: cukup untuk menguji seluruh aritmetika glyph tanpa font
    /// dan tanpa GPU. Bukti bahwa jalur teks di renderer benar-benar hanya
    /// berbicara lewat `GlyphSource` (§3.2) — kalau ia diam-diam butuh
    /// `silka-text`, test ini tidak akan bisa ditulis.
    #[derive(Debug, Default)]
    struct AtlasPalsu {
        ukuran: u32,
        piksel: Vec<u8>,
        letak: HashMap<GlyphImageId, GlyphPlacement>,
        dirty: Option<AtlasRegion>,
    }

    impl AtlasPalsu {
        fn baru(ukuran: u32) -> Self {
            Self {
                ukuran,
                piksel: vec![0; (ukuran * ukuran) as usize],
                letak: HashMap::new(),
                dirty: None,
            }
        }

        fn taruh(&mut self, raw: u32, region: AtlasRegion) -> GlyphImageId {
            let id = GlyphImageId::from_raw(raw);
            self.letak
                .insert(id, GlyphPlacement::new(GlyphFormat::Mask, region));
            self.dirty = Some(region);
            id
        }
    }

    impl GlyphSource for AtlasPalsu {
        fn atlas_size(&self, format: GlyphFormat) -> u32 {
            match format {
                GlyphFormat::Mask => self.ukuran,
                GlyphFormat::Color => 0,
            }
        }

        fn atlas_pixels(&self, format: GlyphFormat) -> &[u8] {
            match format {
                GlyphFormat::Mask => &self.piksel,
                GlyphFormat::Color => &[],
            }
        }

        fn take_dirty(&mut self, format: GlyphFormat) -> Option<AtlasRegion> {
            match format {
                GlyphFormat::Mask => self.dirty.take(),
                GlyphFormat::Color => None,
            }
        }

        fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement> {
            self.letak.get(&image).copied()
        }
    }

    fn kartu(style: CornerStyle) -> Quad {
        Quad::new(Rect::new(20.0, 40.0, 200.0, 100.0))
            .background(Color::hex(0xFFFFFF))
            .corners(Corners::uniform(16.0, style))
    }

    fn instances(scene: &Scene) -> Vec<QuadInstance> {
        draw_list_from_scene(scene, ColorSpace::Srgb, 1.0, &NoGlyphs)
            .instances()
            .to_vec()
    }

    fn instances_teks(scene: &Scene, scale: f32, atlas: &AtlasPalsu) -> Vec<QuadInstance> {
        draw_list_from_scene(scene, ColorSpace::Srgb, scale, atlas)
            .instances()
            .to_vec()
    }

    fn batches(scene: &Scene) -> Vec<InstanceBatch> {
        draw_list_from_scene(scene, ColorSpace::Srgb, 1.0, &NoGlyphs)
            .batches()
            .to_vec()
    }

    fn kotak(x: f32, y: f32, w: f32, h: f32) -> Quad {
        Quad::new(Rect::new(x, y, w, h)).background(Color::WHITE)
    }

    fn scene_dengan(command: impl Into<Command>) -> Scene {
        let mut s = Scene::new(Color::BLACK);
        s.push(command);
        s
    }

    #[test]
    fn tata_letak_instance_adalah_lima_vec4_tanpa_padding() {
        assert_eq!(QuadInstance::SIZE, 80);
        assert_eq!(core::mem::align_of::<QuadInstance>(), 4);
        let dua = [QuadInstance::default(); 2];
        assert_eq!(as_bytes(&dua).len(), 160);
    }

    #[test]
    fn kotak_dipetakan_ke_pusat_dan_setengah_ukuran() {
        let i = instances(&scene_dengan(kartu(CornerStyle::Arc)));
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].bounds, [120.0, 90.0, 100.0, 50.0]);
        assert_eq!(i[0].params[3], KIND_QUAD);
    }

    #[test]
    fn arc_memakai_radius_apa_adanya_dan_eksponen_dua() {
        let i = instances(&scene_dengan(kartu(CornerStyle::Arc)));
        assert_eq!(i[0].radii, [16.0; 4]);
        assert_eq!(i[0].params[1], 2.0);
    }

    #[test]
    fn squircle_melebarkan_radius_dan_menaikkan_eksponen() {
        let i = instances(&scene_dengan(kartu(CornerStyle::squircle())));
        // 16 × 1.528 — sudut Apple mulai melengkung lebih awal.
        assert!(
            (i[0].radii[0] - 16.0 * 1.528).abs() < 0.05,
            "{:?}",
            i[0].radii
        );
        assert!((i[0].params[1] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn radius_tidak_pernah_melebihi_separuh_sisi_terpendek() {
        // Token `radius_full` (9999) pada pil: harus jadi tepat setengah tinggi,
        // baik di arc maupun setelah dikali faktor squircle.
        for style in [CornerStyle::Arc, CornerStyle::squircle()] {
            let pil = Quad::new(Rect::new(0.0, 0.0, 120.0, 32.0))
                .background(Color::WHITE)
                .corners(Corners::uniform(9999.0, style));
            let i = instances(&scene_dengan(pil));
            assert_eq!(i[0].radii, [16.0; 4], "{style:?}");
        }
    }

    #[test]
    fn radius_per_sudut_urut_tl_tr_br_bl() {
        let q = Quad::new(Rect::new(0.0, 0.0, 100.0, 100.0))
            .background(Color::WHITE)
            .corners(Corners::new(
                silka_paint::CornerRadii {
                    top_left: 1.0,
                    top_right: 2.0,
                    bottom_right: 3.0,
                    bottom_left: 4.0,
                },
                CornerStyle::Arc,
            ));
        let i = instances(&scene_dengan(q));
        assert_eq!(i[0].radii, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn border_dibatasi_agar_tidak_melipat_bentuk() {
        let q = Quad::new(Rect::new(0.0, 0.0, 100.0, 20.0))
            .background(Color::WHITE)
            .border(50.0, Color::BLACK);
        let i = instances(&scene_dengan(q));
        assert_eq!(i[0].params[0], 10.0);
    }

    #[test]
    fn target_srgb_menerima_warna_linear() {
        let q = Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).background(Color::srgb(0.5, 0.5, 0.5));
        let s = scene_dengan(q);
        let linear = draw_list_from_scene(&s, ColorSpace::Linear, 1.0, &NoGlyphs);
        let linear = linear.instances();
        let apa_adanya = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        let apa_adanya = apa_adanya.instances();
        assert!((linear[0].background[0] - 0.214_041).abs() < 1e-4);
        assert!((apa_adanya[0].background[0] - 0.5).abs() < 1e-6);
        // Alpha tidak pernah ikut dilinearkan.
        assert_eq!(linear[0].background[3], 1.0);
    }

    #[test]
    fn perintah_tak_terlihat_tidak_pernah_sampai_ke_gpu() {
        let mut s = Scene::new(Color::BLACK);
        // Kotak transparan tanpa border.
        s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)));
        // Kotak berukuran nol.
        s.push(Quad::new(Rect::new(0.0, 0.0, 0.0, 10.0)).background(Color::WHITE));
        // Border setebal nol dengan warna terlihat.
        s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).border(0.0, Color::WHITE));
        assert!(instances(&s).is_empty());
    }

    #[test]
    fn kotak_transparan_dengan_border_tetap_digambar() {
        let q = Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).border(1.0, Color::WHITE);
        assert_eq!(instances(&scene_dengan(q)).len(), 1);
    }

    #[test]
    fn bayangan_ganda_menjadi_dua_instance_di_belakang_kotak() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(
            kartu(CornerStyle::squircle()),
            ShadowPair::new(
                Shadow::new(Color::BLACK.with_alpha(0.08), 40.0).offset(0.0, 12.0),
                Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
            ),
        );
        let i = instances(&s);
        assert_eq!(i.len(), 3);
        // Ambient: sigma 20, digeser 12 poin ke bawah.
        assert_eq!(i[0].params[2], 20.0);
        assert_eq!(i[0].params[3], KIND_SHADOW);
        assert_eq!(i[0].bounds[1], 90.0 + 12.0);
        // Key: lebih rapat, lebih dekat.
        assert_eq!(i[1].params[2], 6.0);
        assert_eq!(i[1].bounds[1], 90.0 + 4.0);
        // Kotaknya sendiri paling depan dan tanpa blur.
        assert_eq!(i[2].params[2], 0.0);
        assert_eq!(i[2].params[3], KIND_QUAD);
    }

    #[test]
    fn bayangan_mewarisi_eksponen_sudut_kotaknya() {
        for (style, eksponen) in [(CornerStyle::Arc, 2.0), (CornerStyle::squircle(), 4.0)] {
            let mut s = Scene::new(Color::BLACK);
            s.push_shadowed(
                kartu(style),
                ShadowPair::new(
                    Shadow::new(Color::BLACK.with_alpha(0.2), 20.0),
                    Shadow::NONE,
                ),
            );
            let i = instances(&s);
            assert!((i[0].params[1] - eksponen).abs() < 1e-5, "{style:?}");
        }
    }

    #[test]
    fn bayangan_transparan_dibuang() {
        let mut s = Scene::new(Color::BLACK);
        s.push(ShadowQuad::for_quad(
            &kartu(CornerStyle::Arc),
            Shadow::new(Color::TRANSPARENT, 20.0),
        ));
        assert!(instances(&s).is_empty());
    }

    #[test]
    fn scene_kosong_tidak_menghasilkan_instance() {
        assert!(instances(&Scene::new(Color::BLACK)).is_empty());
    }

    // ---- Batch clip ------------------------------------------------------

    #[test]
    fn scene_tanpa_clip_tetap_satu_batch() {
        // Regresi terhadap batching: menambahkan clip TIDAK boleh membuat UI
        // biasa (yang tidak memotong apa pun) memakai lebih dari satu draw.
        let mut s = Scene::new(Color::BLACK);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(kotak(20.0, 0.0, 10.0, 10.0));
        s.push(kotak(40.0, 0.0, 10.0, 10.0));
        assert_eq!(
            batches(&s),
            vec![InstanceBatch {
                clip: None,
                start: 0,
                end: 3
            }]
        );
    }

    #[test]
    fn clip_memecah_daftar_menjadi_tiga_batch_berurutan() {
        let clip = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut s = Scene::new(Color::BLACK);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(Command::PopClip);
        s.push(kotak(60.0, 60.0, 10.0, 10.0));

        assert_eq!(
            batches(&s),
            vec![
                InstanceBatch {
                    clip: None,
                    start: 0,
                    end: 1
                },
                InstanceBatch {
                    clip: Some(clip),
                    start: 1,
                    end: 3
                },
                InstanceBatch {
                    clip: None,
                    start: 3,
                    end: 4
                },
            ],
            "urutan gambar harus terjaga, dan batch baru hanya saat clip berubah"
        );
    }

    #[test]
    fn clip_bersarang_dipulihkan_ke_kotak_induk_setelah_pop() {
        // Inti kenapa backend tetap butuh TUMPUKAN meski irisannya sudah
        // dilakukan core: `PopClip` tidak membawa kotak induknya. Tanpa
        // tumpukan, kotak ketiga akan ikut terpotong oleh viewport dalam.
        let luar = Rect::new(0.0, 0.0, 100.0, 100.0);
        let dalam = Rect::new(0.0, 0.0, 20.0, 20.0); // sudah = luar ∩ dalam
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(luar));
        s.push(kotak(0.0, 0.0, 200.0, 200.0));
        s.push(Command::PushClip(dalam));
        s.push(kotak(0.0, 0.0, 200.0, 200.0));
        s.push(Command::PopClip);
        s.push(kotak(0.0, 0.0, 200.0, 200.0));
        s.push(Command::PopClip);

        let b = batches(&s);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].clip, Some(luar));
        assert_eq!(b[1].clip, Some(dalam));
        assert_eq!(b[2].clip, Some(luar), "clip induk harus kembali berlaku");
    }

    #[test]
    fn clip_yang_sama_berturut_turut_tidak_memecah_batch() {
        // Dua scroll view bersaudara dengan viewport identik: batch-nya boleh
        // menyatu, tapi HANYA karena kotaknya benar-benar sama.
        let clip = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(Command::PopClip);
        s.push(Command::PushClip(clip));
        s.push(kotak(20.0, 0.0, 10.0, 10.0));
        s.push(Command::PopClip);
        assert_eq!(batches(&s).len(), 1);
    }

    #[test]
    fn clip_kosong_membuang_isinya_sebelum_menyentuh_gpu() {
        // Viewport yang menyusut jadi nol: tidak ada satu piksel pun yang bisa
        // lolos, jadi instance-nya tidak perlu diunggah sama sekali.
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(10.0, 10.0, 0.0, 30.0)));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(Command::PopClip);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));

        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.instances().len(), 1);
        assert_eq!(list.batches().len(), 1);
        assert_eq!(list.batches()[0].clip, None);
    }

    #[test]
    fn pembungkus_clip_tanpa_isi_tidak_menyisakan_batch() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 50.0, 50.0)));
        s.push(Command::PopClip);
        assert!(batches(&s).is_empty());
    }

    #[test]
    fn glyph_di_dalam_clip_ikut_batch_yang_sama() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(9, AtlasRegion::new(0, 0, 8, 8));
        let clip = Rect::new(0.0, 0.0, 40.0, 4.0);
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(0.0, 0.0, 8.0, 8.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(run);
        s.push(Command::PopClip);

        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &atlas);
        assert_eq!(list.instances().len(), 2);
        assert_eq!(
            list.batches(),
            [InstanceBatch {
                clip: Some(clip),
                start: 0,
                end: 2
            }]
        );
    }

    #[test]
    fn instance_tak_terlihat_tidak_membuka_batch() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 50.0, 50.0)));
        // Transparan: dibuang, jadi batch clip ini tidak pernah dibuka.
        s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)));
        s.push(Command::PopClip);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        assert_eq!(
            batches(&s),
            vec![InstanceBatch {
                clip: None,
                start: 0,
                end: 1
            }]
        );
    }

    #[test]
    fn daftar_dipakai_ulang_tanpa_menyisakan_clip_frame_sebelumnya() {
        // Tumpukan clip harus ikut direset: kalau tidak, frame berikutnya
        // mewarisi viewport frame lalu dan seluruh UI ikut terpotong.
        let mut list = DrawList::default();
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(Command::PopClip);
        fill_draw_list(&s, ColorSpace::Srgb, 1.0, &NoGlyphs, &mut list);
        assert_eq!(
            list.batches()[0].clip,
            Some(Rect::new(0.0, 0.0, 10.0, 10.0))
        );

        let mut s2 = Scene::new(Color::BLACK);
        s2.push(kotak(0.0, 0.0, 100.0, 100.0));
        fill_draw_list(&s2, ColorSpace::Srgb, 1.0, &NoGlyphs, &mut list);
        assert_eq!(
            list.batches(),
            [InstanceBatch {
                clip: None,
                start: 0,
                end: 1
            }]
        );
    }

    // ---- Jalur glyph -----------------------------------------------------

    fn scene_teks(atlas: &mut AtlasPalsu, warna: Color) -> Scene {
        let id = atlas.taruh(1, AtlasRegion::new(8, 16, 6, 10));
        let mut run = GlyphRun::new(warna);
        run.push(Glyph::new(id, Rect::new(10.0, 20.0, 6.0, 10.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        s
    }

    #[test]
    fn glyph_run_menjadi_quad_bertekstur_dengan_uv_dari_atlas() {
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::WHITE);
        let i = instances_teks(&s, 1.0, &atlas);

        assert_eq!(i.len(), 1, "satu glyph = satu instance");
        assert_eq!(i[0].params[3], KIND_GLYPH);
        assert_eq!(i[0].params[0], ATLAS_MASK);
        // Kotak tujuan: pusat (13, 25), setengah ukuran (3, 5).
        assert_eq!(i[0].bounds, [13.0, 25.0, 3.0, 5.0]);
        // UV = kotak atlas dinormalkan terhadap sisi atlas 64 px.
        assert_eq!(
            i[0].radii,
            [8.0 / 64.0, 16.0 / 64.0, 14.0 / 64.0, 26.0 / 64.0]
        );
    }

    #[test]
    fn warna_teks_datang_dari_run_bukan_dari_atlas() {
        // Satu bitmap yang sama harus bisa dipakai untuk warna token apa pun —
        // itulah alasan atlas mask hanya menyimpan cakupan.
        let mut atlas = AtlasPalsu::baru(64);
        let label = Color::hex(0xFF3B30);
        let s = scene_teks(&mut atlas, label);
        let i = instances_teks(&s, 1.0, &atlas);
        assert_eq!(i[0].background, label.components());

        let linear = draw_list_from_scene(&s, ColorSpace::Linear, 1.0, &atlas);
        let linear = linear.instances();
        assert_eq!(linear[0].background, label.to_linear());
    }

    #[test]
    fn teks_transparan_tidak_pernah_sampai_ke_gpu() {
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::TRANSPARENT);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn kotak_glyph_disetel_ke_grid_piksel_fisik_pada_layar_2x() {
        // Kunci ketajaman di Retina: satu texel harus jatuh tepat pada satu
        // piksel layar. Kotak logis 0,3 pt di scale 2 dibulatkan ke piksel
        // fisik terdekat, dan lebarnya persis selebar bitmap di atlas.
        let mut atlas = AtlasPalsu::baru(128);
        let id = atlas.taruh(2, AtlasRegion::new(0, 0, 13, 21));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(10.3, 20.4, 6.5, 10.5)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);

        let i = instances_teks(&s, 2.0, &atlas);
        let (cx, cy, hw, hh) = (
            i[0].bounds[0],
            i[0].bounds[1],
            i[0].bounds[2],
            i[0].bounds[3],
        );
        let fisik = |v: f32| v * 2.0;
        // Ukuran fisik = ukuran bitmap, persis.
        assert!((fisik(hw * 2.0) - 13.0).abs() < 1e-4, "{hw}");
        assert!((fisik(hh * 2.0) - 21.0).abs() < 1e-4, "{hh}");
        // Tepi kiri/atas jatuh di piksel fisik bulat (round(10,3×2) = 21).
        let x0 = fisik(cx - hw);
        let y0 = fisik(cy - hh);
        assert!((x0 - 21.0).abs() < 1e-3, "{x0}");
        assert!((y0 - 41.0).abs() < 1e-3, "{y0}");
        assert_eq!(x0.fract(), 0.0);
        assert_eq!(y0.fract(), 0.0);
    }

    #[test]
    fn id_glyph_yang_sudah_hangus_dilewatkan_bukan_digambar_asal() {
        let atlas = AtlasPalsu::baru(64);
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(
            GlyphImageId::from_raw(404),
            Rect::new(0.0, 0.0, 6.0, 10.0),
        ));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn tanpa_sumber_atlas_teks_tidak_menghasilkan_piksel() {
        // Kontrol negatif yang sama dengan uji rasterisasi headless.
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::WHITE);
        assert!(draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs).is_empty());
    }

    #[test]
    fn clip_memotong_kotak_dan_uv_secara_proporsional() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(3, AtlasRegion::new(0, 0, 16, 16));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(0.0, 0.0, 16.0, 16.0)));
        // Setengah kanan dipotong.
        let run = run.clip(Rect::new(0.0, 0.0, 8.0, 16.0));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);

        let i = instances_teks(&s, 1.0, &atlas);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].bounds, [4.0, 8.0, 4.0, 8.0], "kotak ikut terpotong");
        // UV horizontal ikut menyusut setengah; vertikal utuh.
        assert!(
            (i[0].radii[2] - 8.0 / 64.0).abs() < 1e-6,
            "{:?}",
            i[0].radii
        );
        assert!(
            (i[0].radii[3] - 16.0 / 64.0).abs() < 1e-6,
            "{:?}",
            i[0].radii
        );
    }

    #[test]
    fn glyph_di_luar_clip_tidak_digambar_sama_sekali() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(4, AtlasRegion::new(0, 0, 8, 8));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(100.0, 0.0, 8.0, 8.0)));
        let run = run.clip(Rect::new(0.0, 0.0, 40.0, 20.0));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn urutan_gambar_terjaga_antara_kotak_dan_teks() {
        // Inilah yang membuat teks berada DI ATAS latarnya: instance keluar
        // seurut perintah scene, dan semuanya satu draw call.
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(5, AtlasRegion::new(0, 0, 4, 4));
        let mut s = Scene::new(Color::BLACK);
        s.push(Quad::new(Rect::new(0.0, 0.0, 50.0, 50.0)).background(Color::WHITE));
        let mut run = GlyphRun::new(Color::hex(0x0A84FF));
        run.push(Glyph::new(id, Rect::new(4.0, 4.0, 4.0, 4.0)));
        s.push(run);
        s.push(Quad::new(Rect::new(60.0, 0.0, 10.0, 10.0)).background(Color::WHITE));

        let jenis: Vec<f32> = instances_teks(&s, 1.0, &atlas)
            .iter()
            .map(|i| i.params[3])
            .collect();
        assert_eq!(jenis, vec![KIND_QUAD, KIND_GLYPH, KIND_QUAD]);
    }

    #[test]
    fn satu_run_banyak_glyph_menjadi_satu_batch_berurutan() {
        let mut atlas = AtlasPalsu::baru(64);
        let a = atlas.taruh(6, AtlasRegion::new(0, 0, 5, 9));
        let b = atlas.taruh(7, AtlasRegion::new(6, 0, 5, 9));
        let mut run = GlyphRun::with_capacity(Color::WHITE, 2);
        run.push(Glyph::new(a, Rect::new(0.0, 0.0, 5.0, 9.0)));
        run.push(Glyph::new(b, Rect::new(6.0, 0.0, 5.0, 9.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);

        let i = instances_teks(&s, 1.0, &atlas);
        assert_eq!(i.len(), 2);
        assert!(i.iter().all(|x| x.params[3] == KIND_GLYPH));
        assert!(i[1].bounds[0] > i[0].bounds[0], "urut kiri ke kanan");
        // Warna sama = satu batch: tidak ada apa pun yang memisahkan keduanya.
        assert_eq!(i[0].background, i[1].background);
    }

    #[test]
    fn glyph_tanpa_piksel_tidak_menghasilkan_instance() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(8, AtlasRegion::new(0, 0, 0, 0));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(0.0, 0.0, 0.0, 0.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn scale_factor_ngawur_tidak_membuat_kotak_nan() {
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::WHITE);
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let i = instances_teks(&s, scale, &atlas);
            assert_eq!(i.len(), 1, "scale {scale}");
            assert!(i[0].bounds.iter().all(|v| v.is_finite()), "scale {scale}");
        }
    }
}
