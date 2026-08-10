//! Headless **text** rasterization tests: do glyphs really reach the GPU?
//!
//! This patches the most expensive hole in the render path: `Command::GlyphRun`
//! used to be silently dropped by the backend, and not a single test could
//! catch it — every text test stopped on the CPU side (atlas filled, `GlyphRun`
//! built) and every GPU test only drew boxes. So what is proven here is the
//! **pixels**:
//!
//! 1. a scene containing text produces text pixels inside the text box;
//! 2. the same scene **without** the `GlyphRun` produces **zero** pixels in
//!    that box (a negative control — without it, any number could be coming
//!    from the background or a panel);
//! 3. a scene containing text rendered **without an atlas source** is zero too
//!    — proving those pixels really do come from the atlas and not by accident;
//! 4. text is drawn **above** the box preceding it, not painted over;
//! 5. its color comes from the draw command (a theme token), not from the
//!    atlas;
//! 6. on a 2× display glyphs really are rasterized at screen resolution.
//!
//! Everything uses the **bundled** font (`TextEngine::bundled_only`) so results
//! in CI do not depend on whichever fonts happen to be installed
//! (REKOMENDASI §9.5).
//!
//! Without a GPU adapter the tests are skipped with a message — a false failure
//! in CI costs far more than one absent test.

use silka_paint::{Color, Point, Quad, Rect, Scene, Size};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};
use silka_text::{TextConstraints, TextEngine, TextStyle};

/// A 320×120 point canvas — it fits one line of large text with a roomy margin.
const LEBAR: f32 = 320.0;
const TINGGI: f32 = 120.0;
/// The top-left corner of the text block.
const ORIGIN: Point = Point::new(16.0, 24.0);
/// The test font size: large enough that the pixel count is well above noise.
const UKURAN: f32 = 32.0;
const CONTOH: &str = "Halo dunia";

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("dilewati: tidak ada GPU untuk render headless ({e})");
            None
        }
    }
}

fn kanvas(gpu: &Gpu, scale: f64) -> OffscreenTarget {
    let geometry = SurfaceGeometry::from_logical(Size::new(LEBAR, TINGGI), scale);
    OffscreenTarget::new(gpu, geometry).expect("target headless gagal dibuat")
}

fn mesin(scale: f32) -> TextEngine {
    let mut e = TextEngine::bundled_only();
    e.set_scale_factor(scale);
    e
}

fn gaya() -> TextStyle {
    TextStyle::new().size(UKURAN).single_line()
}

/// A scene holding one line of text, together with the logical box it occupies.
fn scene_teks(mesin: &mut TextEngine, warna: Color) -> (Scene, Rect) {
    let mut scene = Scene::new(Color::BLACK);
    let layout = mesin.layout(CONTOH, &gaya(), TextConstraints::UNBOUNDED);
    let run = mesin.rasterize(&layout, ORIGIN, warna);
    let kotak = run.bounds().expect("teks harus punya glyph");
    scene.push(run);
    (scene, kotak)
}

/// Pixels within the logical box that are not background (background = solid
/// black).
///
/// The threshold of 24 leaves room for very faint anti-aliased edges, but sits
/// far below actual glyph coverage.
fn piksel_teks(img: &Rgba8Image, kotak: Rect, scale: f64) -> usize {
    let mut n = 0;
    let (x0, y0, x1, y1) = batas_fisik(kotak, scale, img);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.pixel(x, y);
            if p[0] as u32 + p[1] as u32 + p[2] as u32 > 24 {
                n += 1;
            }
        }
    }
    n
}

fn batas_fisik(kotak: Rect, scale: f64, img: &Rgba8Image) -> (u32, u32, u32, u32) {
    let f = |v: f32| (v as f64 * scale).round().max(0.0) as u32;
    (
        f(kotak.min_x()).min(img.width()),
        f(kotak.min_y()).min(img.height()),
        f(kotak.max_x()).min(img.width()),
        f(kotak.max_y()).min(img.height()),
    )
}

#[test]
fn teks_benar_benar_menghasilkan_piksel_di_gpu() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    let (scene, kotak) = scene_teks(&mut mesin, Color::WHITE);
    let img = target
        .render_with_glyphs(&gpu, &scene, &mut mesin)
        .expect("render teks gagal");

    let terisi = piksel_teks(&img, kotak, 1.0);
    let luas = (kotak.size.width * kotak.size.height) as usize;
    assert!(
        terisi > 200,
        "teks nyaris tidak menghasilkan piksel: {terisi} dari luas {luas}"
    );
    // Text is not a solid block: most of its box stays background.
    assert!(
        terisi < luas * 3 / 4,
        "kotak teks malah tergambar penuh ({terisi} dari {luas}) — bukan glyph"
    );
    // Outside the text box there must be nothing at all.
    assert_eq!(img.pixel(1, 1), [0, 0, 0, 255], "latar ikut ternoda");
}

#[test]
fn scene_tanpa_glyph_run_menghasilkan_nol_piksel_teks() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // The box is taken from a scene that DOES contain text, then measured on a
    // scene containing nothing — the exact same area, and the result must be
    // zero.
    let (_, kotak) = scene_teks(&mut mesin, Color::WHITE);
    let kosong = Scene::new(Color::BLACK);
    let img = target
        .render_with_glyphs(&gpu, &kosong, &mut mesin)
        .expect("render kosong gagal");

    assert_eq!(piksel_teks(&img, kotak, 1.0), 0);
}

#[test]
fn tanpa_sumber_atlas_teks_tidak_pernah_muncul() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    let (scene, kotak) = scene_teks(&mut mesin, Color::WHITE);
    // `render` without an atlas source: the glyph commands exist, the bitmaps
    // do not.
    let img = target.render(&gpu, &scene).expect("render gagal");
    assert_eq!(
        piksel_teks(&img, kotak, 1.0),
        0,
        "piksel teks muncul dari mana? seharusnya tidak ada atlas sama sekali"
    );
}

#[test]
fn teks_digambar_di_atas_kotak_yang_mendahuluinya() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // A solid white panel first, dark text after — if the draw order breaks,
    // the text is painted over and this box comes out uniformly white.
    let layout = mesin.layout(CONTOH, &gaya(), TextConstraints::UNBOUNDED);
    let run = mesin.rasterize(&layout, ORIGIN, Color::BLACK);
    let kotak = run.bounds().expect("teks harus punya glyph");

    let mut scene = Scene::new(Color::BLACK);
    scene.push(Quad::new(Rect::new(0.0, 0.0, LEBAR, TINGGI)).background(Color::WHITE));
    scene.push(run);

    let img = target
        .render_with_glyphs(&gpu, &scene, &mut mesin)
        .expect("render gagal");

    let (x0, y0, x1, y1) = batas_fisik(kotak, 1.0, &img);
    let mut gelap = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if img.pixel(x, y)[0] < 128 {
                gelap += 1;
            }
        }
    }
    assert!(
        gelap > 200,
        "teks tertimpa panel: hanya {gelap} piksel gelap"
    );
    // Outside the text box the panel stays cleanly white.
    assert_eq!(img.pixel(1, 1), [255, 255, 255, 255]);
}

#[test]
fn warna_teks_datang_dari_token_bukan_dari_atlas() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // Two colors, one atlas: the very same bitmap must serve both.
    let merah = Color::hex(0xFF3B30);
    let (scene_merah, kotak) = scene_teks(&mut mesin, merah);
    let img_merah = target
        .render_with_glyphs(&gpu, &scene_merah, &mut mesin)
        .expect("render merah gagal");
    let glyph_sesudah_frame_1 = mesin.glyphs().len();

    let (scene_putih, _) = scene_teks(&mut mesin, Color::WHITE);
    let img_putih = target
        .render_with_glyphs(&gpu, &scene_putih, &mut mesin)
        .expect("render putih gagal");

    assert_eq!(
        mesin.glyphs().len(),
        glyph_sesudah_frame_1,
        "ganti warna tidak boleh merasterisasi ulang glyph"
    );

    let (x0, y0, x1, y1) = batas_fisik(kotak, 1.0, &img_merah);
    let mut piksel_merah = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img_merah.pixel(x, y);
            if p[0] > 128 {
                assert!(
                    p[0] > p[2] + 40,
                    "piksel teks tidak semerah tokennya: {p:?}"
                );
                piksel_merah += 1;
            }
        }
    }
    assert!(piksel_merah > 100, "teks merah tidak tergambar");
    assert_ne!(
        img_merah.pixels(),
        img_putih.pixels(),
        "warna run tidak berpengaruh — warna pasti terkunci di tempat lain"
    );
}

#[test]
fn layar_2x_merasterisasi_teks_pada_resolusi_layar() {
    let Some(gpu) = gpu() else { return };

    let mut satu = mesin(1.0);
    let mut target_1x = kanvas(&gpu, 1.0);
    let (scene_1x, kotak) = scene_teks(&mut satu, Color::WHITE);
    let img_1x = target_1x
        .render_with_glyphs(&gpu, &scene_1x, &mut satu)
        .expect("render 1x gagal");

    let mut dua = mesin(2.0);
    let mut target_2x = kanvas(&gpu, 2.0);
    let (scene_2x, kotak_2x) = scene_teks(&mut dua, Color::WHITE);
    let img_2x = target_2x
        .render_with_glyphs(&gpu, &scene_2x, &mut dua)
        .expect("render 2x gagal");

    assert_eq!(img_2x.width(), img_1x.width() * 2);
    // The logical box is nearly identical — what doubles is the pixels.
    assert!((kotak_2x.size.width - kotak.size.width).abs() < 2.0);

    let n1 = piksel_teks(&img_1x, kotak, 1.0);
    let n2 = piksel_teks(&img_2x, kotak_2x, 2.0);
    assert!(
        n2 > n1 * 2,
        "teks 2x tidak lebih rinci: {n1} px pada 1x, {n2} px pada 2x"
    );

    // Crisp, not soft: glyphs at 2x still have a fully solid core.
    let (x0, y0, x1, y1) = batas_fisik(kotak_2x, 2.0, &img_2x);
    let mut pekat = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if img_2x.pixel(x, y)[0] > 250 {
                pekat += 1;
            }
        }
    }
    assert!(
        pekat > 100,
        "tidak ada piksel teks yang benar-benar pekat di 2x ({pekat}) — sampling meleset dari grid texel"
    );
}

#[test]
fn frame_kedua_memakai_atlas_yang_sudah_terunggah() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    let (scene, kotak) = scene_teks(&mut mesin, Color::WHITE);
    let frame1 = target
        .render_with_glyphs(&gpu, &scene, &mut mesin)
        .expect("frame 1 gagal");

    // After the first frame nothing is dirty any more: if the second frame is
    // still correct, the texture really did persist and was not re-uploaded.
    assert!(
        silka_paint::GlyphSource::take_dirty(&mut mesin, silka_paint::GlyphFormat::Mask).is_none(),
        "atlas masih menandai dirty setelah diunggah — akan diunggah ulang tiap frame"
    );

    let frame2 = target
        .render_with_glyphs(&gpu, &scene, &mut mesin)
        .expect("frame 2 gagal");
    assert_eq!(frame1.pixels(), frame2.pixels());
    assert!(piksel_teks(&frame2, kotak, 1.0) > 200);
}

#[test]
fn glyph_baru_di_frame_berikutnya_ikut_terunggah() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // The first frame fills the atlas with one set of glyphs…
    let (scene_a, kotak_a) = scene_teks(&mut mesin, Color::WHITE);
    let img_a = target
        .render_with_glyphs(&gpu, &scene_a, &mut mesin)
        .expect("frame A gagal");
    assert!(piksel_teks(&img_a, kotak_a, 1.0) > 200);

    // …the second frame uses characters that have never appeared before. If
    // the incremental upload picks the wrong rect, this new text comes out
    // blank.
    let mut scene_b = Scene::new(Color::BLACK);
    let layout = mesin.layout("XYZ&#%", &gaya(), TextConstraints::UNBOUNDED);
    let run = mesin.rasterize(&layout, ORIGIN, Color::WHITE);
    let kotak_b = run.bounds().expect("ada glyph");
    scene_b.push(run);

    let img_b = target
        .render_with_glyphs(&gpu, &scene_b, &mut mesin)
        .expect("frame B gagal");
    assert!(
        piksel_teks(&img_b, kotak_b, 1.0) > 200,
        "glyph baru tidak ikut terunggah"
    );
}

/// A synthetic atlas that can be forced to change size — the "atlas fills up
/// and gets rebuilt" path happens too rarely with real fonts to rely on as a
/// test, even though it is exactly where the GPU texture must be recreated
/// **and** the bind group rebuilt.
struct AtlasBuatan {
    size: u32,
    piksel: Vec<u8>,
    region: silka_paint::AtlasRegion,
    dirty: Option<silka_paint::AtlasRegion>,
}

impl AtlasBuatan {
    fn baru(size: u32, region: silka_paint::AtlasRegion) -> Self {
        let mut piksel = vec![0u8; (size * size) as usize];
        for y in region.y..region.max_y() {
            for x in region.x..region.max_x() {
                piksel[(y * size + x) as usize] = 0xFF;
            }
        }
        Self {
            size,
            piksel,
            region,
            dirty: Some(region),
        }
    }
}

impl silka_paint::GlyphSource for AtlasBuatan {
    fn atlas_size(&self, format: silka_paint::GlyphFormat) -> u32 {
        match format {
            silka_paint::GlyphFormat::Mask => self.size,
            silka_paint::GlyphFormat::Color => 0,
        }
    }

    fn atlas_pixels(&self, format: silka_paint::GlyphFormat) -> &[u8] {
        match format {
            silka_paint::GlyphFormat::Mask => &self.piksel,
            silka_paint::GlyphFormat::Color => &[],
        }
    }

    fn take_dirty(&mut self, format: silka_paint::GlyphFormat) -> Option<silka_paint::AtlasRegion> {
        match format {
            silka_paint::GlyphFormat::Mask => self.dirty.take(),
            silka_paint::GlyphFormat::Color => None,
        }
    }

    fn placement(&self, _image: silka_paint::GlyphImageId) -> Option<silka_paint::GlyphPlacement> {
        Some(silka_paint::GlyphPlacement::new(
            silka_paint::GlyphFormat::Mask,
            self.region,
        ))
    }
}

#[test]
fn tekstur_dibuat_ulang_saat_atlas_berganti_ukuran() {
    use silka_paint::{AtlasRegion, Glyph, GlyphImageId, GlyphRun};

    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    let kotak = Rect::new(10.0, 10.0, 8.0, 8.0);
    let mut scene = Scene::new(Color::BLACK);
    let mut run = GlyphRun::new(Color::WHITE);
    run.push(Glyph::new(GlyphImageId::from_raw(1), kotak));
    scene.push(run);

    // A small atlas first…
    let mut atlas = AtlasBuatan::baru(16, AtlasRegion::new(0, 0, 8, 8));
    let kecil = target
        .render_with_glyphs(&gpu, &scene, &mut atlas)
        .expect("render atlas kecil gagal");
    assert_eq!(kecil.pixel(12, 12), [255, 255, 255, 255]);

    // …then the atlas is rebuilt larger, with the glyph somewhere else. If the
    // texture/bind group are not updated too, this box comes out black.
    let mut atlas = AtlasBuatan::baru(64, AtlasRegion::new(32, 40, 8, 8));
    let besar = target
        .render_with_glyphs(&gpu, &scene, &mut atlas)
        .expect("render atlas besar gagal");
    assert_eq!(besar.pixel(12, 12), [255, 255, 255, 255]);
    assert_eq!(besar.pixel(1, 1), [0, 0, 0, 255], "latar tetap latar");
}

#[test]
fn atlas_yang_tumbuh_tidak_membuat_teks_hilang() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // Many different font sizes = many bitmaps = the atlas is forced to grow.
    // What is being tested: after the texture is recreated, the bind group is
    // rebuilt too and the contents are uploaded in full — otherwise text
    // vanishes without an error.
    let ukuran_awal = mesin.glyphs().mask_atlas().size();
    for i in 0..48 {
        let gaya = TextStyle::new().size(8.0 + i as f32 * 1.5).single_line();
        let layout = mesin.layout(
            "Mengukur atlas sampai penuh",
            &gaya,
            TextConstraints::UNBOUNDED,
        );
        let _ = mesin.rasterize(&layout, ORIGIN, Color::WHITE);
    }
    let tumbuh = mesin.glyphs().mask_atlas().size() > ukuran_awal;

    let (scene, kotak) = scene_teks(&mut mesin, Color::WHITE);
    let img = target
        .render_with_glyphs(&gpu, &scene, &mut mesin)
        .expect("render setelah atlas tumbuh gagal");
    assert!(
        piksel_teks(&img, kotak, 1.0) > 200,
        "teks hilang setelah atlas {} (ukuran {} → {})",
        if tumbuh { "tumbuh" } else { "tetap" },
        ukuran_awal,
        mesin.glyphs().mask_atlas().size()
    );
}
