//! Headless rasterization tests: what actually comes out of the SDF shader.
//!
//! The unit tests inside the crate guard the *data* sent to the GPU; this file
//! guards the *result* — that a squircle really does differ from an arc, that
//! the border sits inside the edge, and that a double shadow really does darken
//! the background (REKOMENDASI §9.5: headless rendering for CI).
//!
//! If the machine running the tests has no GPU adapter at all (a CI container
//! without drivers), the tests are **skipped with a message** rather than
//! failing: a false failure in CI costs far more than one absent test.

use silka_paint::{Color, CornerStyle, Corners, Quad, Rect, Scene, Shadow, ShadowPair, Size};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};

/// A 256×256 logical-point canvas (scale 1.0) — large enough that the corner
/// geometry difference is measurable in whole pixels.
const SISI: u32 = 256;
/// A 224×224 card centered on the canvas, leaving 16 points of shadow margin.
const MARGIN: f32 = 16.0;
/// The nominal radius used by every shape test.
const RADIUS: f32 = 48.0;

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("dilewati: tidak ada GPU untuk render headless ({e})");
            None
        }
    }
}

fn kanvas(gpu: &Gpu) -> OffscreenTarget {
    OffscreenTarget::new(gpu, SurfaceGeometry::new(SISI, SISI, 1.0))
        .expect("target headless gagal dibuat")
}

fn kartu(style: CornerStyle) -> Quad {
    Quad::new(Rect::new(
        MARGIN,
        MARGIN,
        SISI as f32 - MARGIN * 2.0,
        SISI as f32 - MARGIN * 2.0,
    ))
    .background(Color::WHITE)
    .corners(Corners::uniform(RADIUS, style))
}

fn scene_kartu(style: CornerStyle) -> Scene {
    let mut s = Scene::new(Color::BLACK);
    s.push(kartu(style));
    s
}

/// The number of pixels genuinely filled by the card (solid white).
fn piksel_terisi(img: &Rgba8Image) -> usize {
    img.pixels()
        .chunks(4)
        .filter(|p| p[0] > 250 && p[1] > 250 && p[2] > 250)
        .count()
}

fn terang(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

#[test]
fn kotak_digambar_dan_latar_tetap_latar() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu);
    let img = target
        .render(&gpu, &scene_kartu(CornerStyle::Arc))
        .expect("render gagal");

    assert_eq!(img.width(), SISI);
    assert_eq!(img.height(), SISI);
    // The card's center = the fill color.
    assert_eq!(img.pixel(SISI / 2, SISI / 2), [255, 255, 255, 255]);
    // A canvas corner far outside the card = the background color.
    assert_eq!(terang(img.pixel(1, 1)), 0);
}

#[test]
fn squircle_dan_arc_menghasilkan_bentuk_yang_berbeda() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu);
    let squircle = target
        .render(&gpu, &scene_kartu(CornerStyle::squircle()))
        .expect("render squircle gagal");
    let arc = target
        .render(&gpu, &scene_kartu(CornerStyle::Arc))
        .expect("render arc gagal");

    assert_ne!(squircle.pixels(), arc.pixels(), "kedua mode harus berbeda");

    // A superellipse cuts less area off the corner than a circular arc
    // (0.073·R² vs 0.215·R²), even though it starts curving earlier — that is
    // the "Apple corner": fuller at the tip, smoother in transition.
    let luas_squircle = piksel_terisi(&squircle);
    let luas_arc = piksel_terisi(&arc);
    assert!(
        luas_squircle > luas_arc,
        "squircle {luas_squircle} px, arc {luas_arc} px",
    );

    // Both are still rounded boxes, not full rectangles.
    let luas_penuh = (SISI as f32 - MARGIN * 2.0).powi(2) as usize;
    assert!(luas_squircle < luas_penuh, "sudut squircle tidak terpotong");
}

#[test]
fn ujung_diagonal_sudut_squircle_lebih_penuh_daripada_arc() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu);
    let squircle = target
        .render(&gpu, &scene_kartu(CornerStyle::squircle()))
        .expect("render squircle gagal");
    let arc = target
        .render(&gpu, &scene_kartu(CornerStyle::Arc))
        .expect("render arc gagal");

    // A probe point on the top-left corner's diagonal. Analytically the shape
    // boundary sits at:
    //   arc      → 14.06 points from the corner point (r·(1 − 1/√2), r = 48)
    //   squircle → 11.68 points (R = 48·1.528; R − R·2^(−1/4))
    // Pixel (28,28) — 12.5 points from the corner — falls between the two.
    let x = MARGIN as u32 + 12;
    assert_eq!(
        squircle.pixel(x, x),
        [255, 255, 255, 255],
        "squircle harus terisi"
    );
    assert_eq!(
        terang(arc.pixel(x, x)),
        0,
        "arc harus masih kosong di titik itu"
    );
}

#[test]
fn border_berada_di_dalam_tepi_dan_tidak_menutupi_isi() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu);

    let mut scene = Scene::new(Color::BLACK);
    scene.push(
        Quad::new(Rect::new(64.0, 64.0, 128.0, 128.0))
            .background(Color::WHITE)
            // An 8-point red border: thick enough to sample.
            .border(8.0, Color::hex(0xFF0000))
            .corners(Corners::uniform(16.0, CornerStyle::squircle())),
    );
    let img = target.render(&gpu, &scene).expect("render gagal");

    // Just inside the left edge = border.
    let tepi = img.pixel(68, 128);
    assert!(
        tepi[0] > 200 && tepi[1] < 60 && tepi[2] < 60,
        "tepi = {tepi:?}"
    );
    // A few points further in = fill.
    assert_eq!(img.pixel(80, 128), [255, 255, 255, 255]);
    // Outside the box = background.
    assert_eq!(terang(img.pixel(60, 128)), 0);
}

#[test]
fn shadow_ganda_menggelapkan_latar_di_bawah_kartu() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu);

    let quad = Quad::new(Rect::new(64.0, 48.0, 128.0, 128.0))
        .background(Color::WHITE)
        .corners(Corners::uniform(24.0, CornerStyle::squircle()));

    let mut tanpa = Scene::new(Color::hex(0x808080));
    tanpa.push(quad.clone());
    let polos = target.render(&gpu, &tanpa).expect("render gagal");

    let mut dengan = Scene::new(Color::hex(0x808080));
    dengan.push_shadowed(
        quad,
        ShadowPair::new(
            Shadow::new(Color::BLACK.with_alpha(0.30), 40.0).offset(0.0, 12.0),
            Shadow::new(Color::BLACK.with_alpha(0.40), 12.0).offset(0.0, 4.0),
        ),
    );
    let berbayang = target.render(&gpu, &dengan).expect("render gagal");

    // Just below the card (its bottom edge is at y 176): the shadow must be
    // clearly visible.
    let bawah = (128, 180);
    assert!(
        terang(berbayang.pixel(bawah.0, bawah.1)) + 45 < terang(polos.pixel(bawah.0, bawah.1)),
        "bayangan tidak terlihat: {:?} vs {:?}",
        berbayang.pixel(bawah.0, bawah.1),
        polos.pixel(bawah.0, bawah.1),
    );

    // The shadow decays smoothly: brighter the further out, with never a step
    // that reverses direction (banding would show up as a reversal).
    let profil: Vec<u32> = (0..8)
        .map(|i| terang(berbayang.pixel(128, 180 + i * 5)))
        .collect();
    assert!(
        profil.windows(2).all(|w| w[1] >= w[0]),
        "gaussian tidak meluruh monoton: {profil:?}",
    );
    assert!(profil[0] < profil[profil.len() - 1], "{profil:?}");
    assert_eq!(
        berbayang.pixel(2, 2),
        polos.pixel(2, 2),
        "latar jauh dari kartu tidak boleh ikut gelap",
    );

    // The card itself stays opaque: the shadow is drawn behind it.
    assert_eq!(berbayang.pixel(128, 112), [255, 255, 255, 255]);
}

#[test]
fn scene_kosong_hanya_menghasilkan_warna_latar() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu);
    let img = target
        .render(&gpu, &Scene::new(Color::hex(0x1C1C1E)))
        .expect("render gagal");
    // A sRGB↔linear round trip must return the very same bytes.
    for (x, y) in [(0, 0), (SISI / 2, SISI / 2), (SISI - 1, SISI - 1)] {
        assert_eq!(img.pixel(x, y), [0x1C, 0x1C, 0x1E, 255], "di ({x},{y})");
    }
}

#[test]
fn ukuran_logis_ikut_scale_factor() {
    let Some(gpu) = gpu() else { return };
    // 512 physical pixels @2× = 256 logical points: the same card must cover
    // the same proportion of the canvas, just with twice as many pixels.
    let mut retina = OffscreenTarget::new(&gpu, SurfaceGeometry::new(SISI * 2, SISI * 2, 2.0))
        .expect("target retina gagal dibuat");
    assert_eq!(retina.logical_size(), Size::new(SISI as f32, SISI as f32));

    let img = retina
        .render(&gpu, &scene_kartu(CornerStyle::squircle()))
        .expect("render gagal");
    assert_eq!(img.width(), SISI * 2);
    assert_eq!(img.pixel(SISI, SISI), [255, 255, 255, 255]);
    // A 16-point margin = 32 physical pixels: pixel (8,8) is still background.
    assert_eq!(terang(img.pixel(8, 8)), 0);

    let mut satu_x = kanvas(&gpu);
    let img_1x = satu_x
        .render(&gpu, &scene_kartu(CornerStyle::squircle()))
        .expect("render gagal");
    let rasio = piksel_terisi(&img) as f32 / piksel_terisi(&img_1x) as f32;
    assert!((rasio - 4.0).abs() < 0.05, "rasio luas = {rasio}");
}
