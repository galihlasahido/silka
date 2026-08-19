//! **Pixel** tests for the four commands that unblocked the component
//! catalogue: stroke, transform, image, and layer.
//!
//! Structure tests live next to the code that builds the instances (`instance.rs`
//! is full of them, and they need no GPU). What can only be proven here is that
//! the shader really rasterises what those instances describe — the same gap
//! `klip.rs` exists to close for clipping:
//!
//! 1. a **stroke** produces a line of the requested width, and a diagonal one is
//!    not thinner than a horizontal one (the bug the old column rasteriser had);
//! 2. a **round cap** really reaches past its endpoint, and a butt cap does not;
//! 3. a **transform** moves the pixels of a whole subtree, not just its geometry;
//! 4. an **image** shows its own texels, and a coverage mask takes the tint from
//!    its command rather than from the atlas;
//! 5. a **layer** applies group opacity, and a blurred layer bleeds colour beyond
//!    the shape that was drawn into it.
//!
//! Without a GPU adapter the tests are skipped with a message — a false failure
//! in CI costs far more than one absent test.

use silka_paint::{
    Color, ImageAtlas, ImageQuad, Layer, LineCap, LineJoin, NoGlyphs, Point, Quad, Rect, Scene,
    Size, Stroke, Transform,
};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};

/// A 64×64 point canvas at 1× — logical points and physical pixels coincide, so
/// every assertion below can name the pixel it means.
const SISI: f32 = 64.0;

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
    OffscreenTarget::new(
        gpu,
        SurfaceGeometry::from_logical(Size::new(SISI, SISI), 1.0),
    )
    .expect("target headless gagal dibuat")
}

fn render(gpu: &Gpu, scene: &Scene) -> Rgba8Image {
    kanvas(gpu).render(gpu, scene).expect("render gagal")
}

fn render_dengan_gambar(gpu: &Gpu, scene: &Scene, atlas: &mut ImageAtlas) -> Rgba8Image {
    kanvas(gpu)
        .render_with_sources(gpu, scene, &mut NoGlyphs, atlas)
        .expect("render gagal")
}

/// True when a pixel is clearly lit rather than background or a faint AA edge.
fn menyala(img: &Rgba8Image, x: u32, y: u32) -> bool {
    img.pixel(x, y)[0] > 128
}

/// How many lit pixels a vertical column holds — the thickness of a line as the
/// screen actually shows it.
fn tebal_kolom(img: &Rgba8Image, x: u32) -> u32 {
    (0..img.height()).filter(|y| menyala(img, x, *y)).count() as u32
}

fn scene_hitam() -> Scene {
    Scene::new(Color::BLACK)
}

/// One sRGB-encoded channel byte as linear light — the space the sampler blends
/// in, and so the only space where a blend tolerance means anything.
fn linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// ---------------------------------------------------------------------------
// Stroke
// ---------------------------------------------------------------------------

#[test]
fn stroke_menggambar_garis_selebar_yang_diminta() {
    let Some(gpu) = gpu() else { return };
    let mut s = scene_hitam();
    s.push(Stroke::line(
        Point::new(8.0, 32.0),
        Point::new(56.0, 32.0),
        Color::WHITE,
        6.0,
    ));
    let img = render(&gpu, &s);

    // The centre of the line is lit, and its thickness is the width asked for
    // (± one pixel of anti-aliasing at each edge).
    assert!(
        menyala(&img, 32, 32),
        "garisnya tidak tergambar sama sekali"
    );
    let tebal = tebal_kolom(&img, 32);
    assert!((5..=7).contains(&tebal), "tebal {tebal}, seharusnya 6");
    // Well away from the line there is nothing at all.
    assert_eq!(img.pixel(32, 8), [0, 0, 0, 255]);
    assert_eq!(img.pixel(4, 32), [0, 0, 0, 255], "sebelum ujung kiri");
}

#[test]
fn ruas_diagonal_tidak_lebih_tipis_dari_yang_datar() {
    // The exact bug the column rasteriser had: a diagonal drawn as one box per
    // pixel column thins out unless its height is corrected by √(1+m²). A capsule
    // has the right thickness by construction.
    let Some(gpu) = gpu() else { return };

    let mut datar = scene_hitam();
    datar.push(Stroke::line(
        Point::new(8.0, 32.0),
        Point::new(56.0, 32.0),
        Color::WHITE,
        6.0,
    ));
    let mut diagonal = scene_hitam();
    diagonal.push(Stroke::line(
        Point::new(8.0, 8.0),
        Point::new(56.0, 56.0),
        Color::WHITE,
        6.0,
    ));

    let a = tebal_kolom(&render(&gpu, &datar), 32);
    let b = tebal_kolom(&render(&gpu, &diagonal), 32);
    // A 45° line covers √2 times more rows for the same perpendicular width.
    assert!(b >= a, "diagonal {b} lebih tipis dari datar {a}");
    assert!(b <= a * 2, "diagonal {b} jauh terlalu gemuk (datar {a})");
}

#[test]
fn cap_bulat_melewati_ujungnya_cap_datar_tidak() {
    let Some(gpu) = gpu() else { return };
    let garis = |cap: LineCap| {
        let mut s = scene_hitam();
        s.push(
            Stroke::line(
                Point::new(20.0, 32.0),
                Point::new(44.0, 32.0),
                Color::WHITE,
                8.0,
            )
            .cap(cap),
        );
        render(&gpu, &s)
    };

    let bulat = garis(LineCap::Round);
    let datar = garis(LineCap::Butt);
    // Two points past the endpoint: inside a round cap (radius 4), outside a
    // butt cap.
    assert!(menyala(&bulat, 46, 32), "cap bulat tidak membulat");
    assert!(!menyala(&datar, 46, 32), "cap datar malah memanjang");
    // Both are lit well inside the segment.
    assert!(menyala(&bulat, 32, 32) && menyala(&datar, 32, 32));
}

#[test]
fn polyline_tersambung_di_simpulnya() {
    // A notch at a join is the classic artefact of drawing segments separately;
    // the round join dot is what fills the wedge.
    let Some(gpu) = gpu() else { return };
    let mut s = scene_hitam();
    let mut garis = Stroke::new(Color::WHITE, 6.0)
        .cap(LineCap::Butt)
        .join(LineJoin::Round);
    garis.extend([
        Point::new(8.0, 48.0),
        Point::new(32.0, 16.0),
        Point::new(56.0, 48.0),
    ]);
    s.push(garis);
    let img = render(&gpu, &s);

    // The apex itself is covered — that is the join.
    assert!(menyala(&img, 32, 17), "sambungan berlubang di puncaknya");
    // And both arms are drawn.
    assert!(menyala(&img, 20, 32));
    assert!(menyala(&img, 44, 32));
}

// ---------------------------------------------------------------------------
// Transform
// ---------------------------------------------------------------------------

#[test]
fn transform_memindahkan_piksel_bukan_hanya_geometri() {
    let Some(gpu) = gpu() else { return };
    let kotak = Rect::new(16.0, 16.0, 32.0, 32.0);

    let mut apa_adanya = scene_hitam();
    apa_adanya.push(Quad::new(kotak).background(Color::WHITE));
    let penuh = render(&gpu, &apa_adanya);
    assert!(
        menyala(&penuh, 20, 20),
        "kontrol: kotak penuh menutupi (20,20)"
    );

    let mut mengecil = scene_hitam();
    mengecil.with_transform(Transform::scale_around(kotak.center(), 0.5, 0.5), |s| {
        s.push(Quad::new(kotak).background(Color::WHITE));
    });
    let kecil = render(&gpu, &mengecil);

    // The centre stays put — that is what "around the centre" means…
    assert!(menyala(&kecil, 32, 32));
    // …and the corner the full-size box covered is now background.
    assert!(
        !menyala(&kecil, 20, 20),
        "transform tidak berpengaruh pada piksel"
    );
    // The scaled box covers 24..40, so its own corner is lit.
    assert!(menyala(&kecil, 26, 26));
}

#[test]
fn rotasi_memiringkan_kotak() {
    let Some(gpu) = gpu() else { return };
    let kotak = Rect::new(24.0, 28.0, 16.0, 8.0);
    let mut s = scene_hitam();
    s.with_transform(
        Transform::rotate_around(kotak.center(), core::f32::consts::FRAC_PI_2),
        |s| {
            s.push(Quad::new(kotak).background(Color::WHITE));
        },
    );
    let img = render(&gpu, &s);
    // A quarter turn swaps the box's axes: what was wide is now tall.
    assert!(menyala(&img, 32, 26), "tidak berputar: atas-bawah kosong");
    assert!(
        !menyala(&img, 22, 32),
        "tidak berputar: kiri-kanan masih terisi"
    );
}

// ---------------------------------------------------------------------------
// Image
// ---------------------------------------------------------------------------

#[test]
fn gambar_menampilkan_texelnya_sendiri() {
    let Some(gpu) = gpu() else { return };
    // A 2×2 bitmap: red, green / blue, white.
    let mut atlas = ImageAtlas::new();
    #[rustfmt::skip]
    let piksel: [u8; 16] = [
        255, 0, 0, 255,   0, 255, 0, 255,
        0, 0, 255, 255,   255, 255, 255, 255,
    ];
    let id = atlas.insert_rgba(2, 2, &piksel).expect("masuk atlas");

    let mut s = scene_hitam();
    s.push(ImageQuad::new(Rect::new(0.0, 0.0, 32.0, 32.0), id));
    let img = render_dengan_gambar(&gpu, &s, &mut atlas);

    // Sampled a quarter and three quarters across, as near the texel centres as
    // whole pixels allow: a pixel centre sits half a pixel off, which is 1/32 of
    // a texel here, so bilinear filtering still blends in a few per cent of each
    // neighbour.
    //
    // The comparison happens in **linear light**, because that is the space the
    // sampler blends in and the readback is sRGB-encoded. The encoding is at its
    // steepest near black: 3% of linear light comes back as 49/255, so a
    // tolerance stated in encoded bytes would have to be ~50 wide — wide enough
    // to accept a genuinely wrong texel. In linear light the same blend is a
    // budget of a few per cent, which is what this test actually means.
    let dekat =
        |a: [u8; 4], b: [u8; 3]| (0..3).all(|i| (linear(a[i]) - linear(b[i])).abs() <= 0.08);
    assert!(dekat(img.pixel(8, 8), [255, 0, 0]), "{:?}", img.pixel(8, 8));
    assert!(
        dekat(img.pixel(24, 8), [0, 255, 0]),
        "{:?}",
        img.pixel(24, 8)
    );
    assert!(
        dekat(img.pixel(8, 24), [0, 0, 255]),
        "{:?}",
        img.pixel(8, 24)
    );
    // Outside the destination box nothing is drawn.
    assert_eq!(img.pixel(48, 48), [0, 0, 0, 255]);
}

#[test]
fn ikon_monokrom_diwarnai_oleh_tokennya() {
    // The reason an icon lives in the same atlas as a photograph: its entry is
    // coverage, and the command's tint is what colours it — so one bitmap serves
    // every token.
    let Some(gpu) = gpu() else { return };
    let mut atlas = ImageAtlas::new();
    let id = atlas.insert_mask(1, 1, &[255]).expect("masuk atlas");

    let biru = Color::hex(0x0A84FF);
    let mut s = scene_hitam();
    s.push(ImageQuad::new(Rect::new(16.0, 16.0, 32.0, 32.0), id).tint(biru));
    let img = render_dengan_gambar(&gpu, &s, &mut atlas);

    let p = img.pixel(32, 32);
    assert!(p[2] > 200, "ikon tidak berwarna token: {p:?}");
    assert!(p[0] < 80, "ikon tidak berwarna token: {p:?}");
}

#[test]
fn sudut_gambar_memotong_bitmapnya() {
    // What makes an avatar a circle: the same superellipse that rounds a box,
    // applied as a mask over the bitmap.
    let Some(gpu) = gpu() else { return };
    let mut atlas = ImageAtlas::new();
    let id = atlas.insert_mask(1, 1, &[255]).expect("masuk atlas");

    let kotak = Rect::new(16.0, 16.0, 32.0, 32.0);
    let mut s = scene_hitam();
    s.push(
        ImageQuad::new(kotak, id)
            .tint(Color::WHITE)
            // radius_full on a square box = a circle.
            .corners(silka_paint::Corners::uniform(
                9999.0,
                silka_paint::CornerStyle::Arc,
            ))
            .normalized(),
    );
    let img = render_dengan_gambar(&gpu, &s, &mut atlas);

    assert!(menyala(&img, 32, 32), "tengah harus terisi");
    assert!(
        !menyala(&img, 17, 17),
        "sudut harus terpotong oleh masker bulat"
    );
}

// ---------------------------------------------------------------------------
// Layer
// ---------------------------------------------------------------------------

#[test]
fn layer_menerapkan_opasitas_kelompok() {
    let Some(gpu) = gpu() else { return };
    let kotak = Rect::new(16.0, 16.0, 32.0, 32.0);

    let mut s = scene_hitam();
    s.with_layer(Layer::new(kotak).opacity(0.5), |s| {
        s.push(Quad::new(kotak).background(Color::WHITE));
    });
    let img = render(&gpu, &s);

    let p = img.pixel(32, 32);
    assert!(p[0] > 40 && p[0] < 250, "bukan setengah transparan: {p:?}");
    assert_eq!(p[0], p[1], "harus tetap kelabu netral: {p:?}");
    assert_eq!(p[1], p[2], "harus tetap kelabu netral: {p:?}");
    // Outside the layer bounds nothing is composited at all.
    assert_eq!(img.pixel(4, 4), [0, 0, 0, 255]);
}

#[test]
fn layer_pass_through_sama_dengan_menggambar_langsung() {
    // The promise that makes it safe to wrap a subtree defensively: a layer with
    // nothing to do must produce the very same pixels, with no texture involved.
    let Some(gpu) = gpu() else { return };
    let kotak = Rect::new(16.0, 16.0, 32.0, 32.0);

    let mut langsung = scene_hitam();
    langsung.push(Quad::new(kotak).background(Color::WHITE));

    let mut berlapis = scene_hitam();
    berlapis.with_layer(Layer::new(kotak), |s| {
        s.push(Quad::new(kotak).background(Color::WHITE));
    });

    assert_eq!(
        render(&gpu, &langsung).pixels(),
        render(&gpu, &berlapis).pixels()
    );
}

#[test]
fn layer_blur_membocorkan_warna_keluar_bentuknya() {
    let Some(gpu) = gpu() else { return };
    let batas = Rect::new(8.0, 8.0, 48.0, 48.0);
    let bentuk = Rect::new(28.0, 28.0, 8.0, 8.0);

    let mut tajam = scene_hitam();
    tajam.push(Quad::new(bentuk).background(Color::WHITE));
    let tajam = render(&gpu, &tajam);
    assert_eq!(
        tajam.pixel(20, 32),
        [0, 0, 0, 255],
        "kontrol: tanpa blur di luar bentuk harus kosong"
    );

    let mut kabur = scene_hitam();
    kabur.with_layer(Layer::new(batas).blur(12.0), |s| {
        s.push(Quad::new(bentuk).background(Color::WHITE));
    });
    let kabur = render(&gpu, &kabur);

    // Colour has spread outside the square that was drawn…
    assert!(
        kabur.pixel(20, 32)[0] > 2,
        "tidak ada blur sama sekali: {:?}",
        kabur.pixel(20, 32)
    );
    // …and the centre has given some of its light away.
    assert!(
        kabur.pixel(32, 32)[0] < 255,
        "pusatnya tidak meredup: {:?}",
        kabur.pixel(32, 32)
    );
    // Still nothing beyond the layer's own bounds.
    assert_eq!(kabur.pixel(2, 2), [0, 0, 0, 255]);
}

#[test]
fn banyak_layer_dan_perintah_biasa_bercampur_tanpa_kehilangan_urutan() {
    // Two sibling layers reuse one target texture. If the second failed to clear
    // it, the first one's pixels would show up inside the second — the bug this
    // test exists to catch.
    let Some(gpu) = gpu() else { return };
    let kiri = Rect::new(4.0, 24.0, 16.0, 16.0);
    let kanan = Rect::new(44.0, 24.0, 16.0, 16.0);

    let mut s = scene_hitam();
    s.with_layer(Layer::new(kiri).opacity(0.9), |s| {
        s.push(Quad::new(kiri).background(Color::WHITE));
    });
    s.push(Quad::new(Rect::new(28.0, 28.0, 8.0, 8.0)).background(Color::WHITE));
    s.with_layer(Layer::new(kanan).opacity(0.9), |s| {
        s.push(Quad::new(kanan).background(Color::WHITE));
    });
    let img = render(&gpu, &s);

    assert!(menyala(&img, 12, 32), "layer pertama hilang");
    assert!(menyala(&img, 32, 32), "perintah di antara dua layer hilang");
    assert!(menyala(&img, 52, 32), "layer kedua hilang");
    // The gaps between them stayed empty: no layer leaked into another.
    assert!(!menyala(&img, 24, 32));
    assert!(!menyala(&img, 40, 32));
}
