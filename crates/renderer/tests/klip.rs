//! **Pixel** tests for `Command::PushClip`/`PopClip`: does what gets clipped
//! really disappear on the GPU?
//!
//! Before this milestone the backend silently ignored clip pairs, and not a
//! single test could catch it: the paint pass already discards commands that
//! fall **entirely** outside the clip, so every CPU-side test passed while
//! **partially** clipped content still drew in full on screen. That is exactly
//! the kind of bug users see as scroll views, lists, and tables leaking out of
//! their viewport.
//!
//! So what is proven here is the pixels, not the structure:
//!
//! 1. a box far larger than the clip leaves pixels only **inside** the clip
//!    rect — and **zero** outside it, even though the quad's geometry extends
//!    well past;
//! 2. text split by a clip loses its bottom half, while the unclipped control
//!    shows that half really is there;
//! 3. after `PopClip` the next drawing is unclipped again;
//! 4. nested clips produce the correct intersection **and** restore the parent
//!    rect after the pop;
//! 5. a clip outside the surface causes neither a panic nor a wgpu validation
//!    error.
//!
//! Without a GPU adapter the tests are skipped with a message — a false failure
//! in CI costs far more than one absent test.

use silka_paint::{Color, Command, Point, Quad, Rect, Scene, Size};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};
use silka_text::{TextConstraints, TextEngine, TextStyle};

/// A 128×128 point canvas — roomy enough for nested clips without touching the
/// edges.
const SISI: f32 = 128.0;

const LATAR: [u8; 4] = [0, 0, 0, 255];
const PUTIH: [u8; 4] = [255, 255, 255, 255];
const MERAH: [u8; 4] = [255, 0, 0, 255];
const BIRU: [u8; 4] = [0, 0, 255, 255];
const HIJAU: [u8; 4] = [0, 255, 0, 255];

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
    let geometry = SurfaceGeometry::from_logical(Size::new(SISI, SISI), scale);
    OffscreenTarget::new(gpu, geometry).expect("target headless gagal dibuat")
}

/// A plain box with no rounded corners: its edges are straight, so every pixel
/// inside is fully covered and can be counted exactly.
fn kotak(rect: Rect, warna: Color) -> Quad {
    Quad::new(rect).background(warna)
}

/// A box far larger than the canvas — its geometry is guaranteed to overflow
/// any clip these tests use.
fn seluruh_kanvas(warna: Color) -> Quad {
    kotak(Rect::new(0.0, 0.0, SISI, SISI), warna)
}

fn jumlah(img: &Rgba8Image, warna: [u8; 4]) -> usize {
    img.pixels().chunks(4).filter(|p| *p == warna).count()
}

#[test]
fn kotak_besar_di_dalam_clip_kecil_hanya_menyisakan_isi_clip() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // A 48×48 clip in the center; the box is 128×128 — nearly seven times the
    // area.
    let clip = Rect::new(32.0, 32.0, 48.0, 48.0);
    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(clip));
    scene.push(seluruh_kanvas(Color::WHITE));
    scene.push(Command::PopClip);

    let img = target.render(&gpu, &scene).expect("render gagal");

    // Inside: fully filled, including the pixels right on the clip edge.
    assert_eq!(img.pixel(56, 56), PUTIH, "tengah clip kosong");
    assert_eq!(
        img.pixel(32, 32),
        PUTIH,
        "piksel sudut kiri-atas clip termakan"
    );
    assert_eq!(
        img.pixel(79, 79),
        PUTIH,
        "piksel sudut kanan-bawah clip termakan"
    );

    // Outside: ZERO, even though the quad covers the whole canvas.
    assert_eq!(img.pixel(31, 56), LATAR, "bocor ke kiri clip");
    assert_eq!(img.pixel(80, 56), LATAR, "bocor ke kanan clip");
    assert_eq!(img.pixel(56, 31), LATAR, "bocor ke atas clip");
    assert_eq!(img.pixel(56, 80), LATAR, "bocor ke bawah clip");
    assert_eq!(img.pixel(0, 0), LATAR);
    assert_eq!(img.pixel(127, 127), LATAR);

    // And the count is exactly the clip's area — not the canvas's.
    assert_eq!(
        jumlah(&img, PUTIH),
        48 * 48,
        "luas terisi harus persis sama dengan luas clip"
    );
}

#[test]
fn clip_dikonversi_ke_piksel_fisik_di_layar_2x() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 2.0);

    // The same clip in LOGICAL POINTS must cover four times as many physical
    // pixels on a 2× display — this is the `SurfaceGeometry` conversion path.
    let clip = Rect::new(32.0, 32.0, 48.0, 48.0);
    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(clip));
    scene.push(seluruh_kanvas(Color::WHITE));
    scene.push(Command::PopClip);

    let img = target.render(&gpu, &scene).expect("render gagal");
    assert_eq!(img.width(), 256);
    assert_eq!(jumlah(&img, PUTIH), 96 * 96);
    assert_eq!(img.pixel(64, 64), PUTIH, "tepi clip fisik termakan");
    assert_eq!(img.pixel(63, 64), LATAR, "bocor satu piksel ke kiri");
    assert_eq!(img.pixel(159, 159), PUTIH);
    assert_eq!(img.pixel(160, 160), LATAR);
}

#[test]
fn teks_yang_terbelah_clip_kehilangan_separuh_bawahnya() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = TextEngine::bundled_only();
    mesin.set_scale_factor(1.0);
    let mut target = kanvas(&gpu, 1.0);

    let gaya = TextStyle::new().size(40.0).single_line();
    let layout = mesin.layout("Halo", &gaya, TextConstraints::UNBOUNDED);
    let run = mesin.rasterize(&layout, Point::new(12.0, 24.0), Color::WHITE);
    let kotak_teks = run.bounds().expect("teks harus punya glyph");

    // Split exactly through the middle of the text box, on a whole-pixel
    // boundary so the scissor's outward rounding does not blur what is being
    // tested.
    let belah = ((kotak_teks.min_y() + kotak_teks.max_y()) * 0.5).ceil();
    let atas = Rect::new(0.0, 0.0, SISI, belah);

    let hitung_baris = |img: &Rgba8Image, y0: u32, y1: u32| {
        let mut n = 0;
        for y in y0..y1 {
            for x in 0..img.width() {
                let p = img.pixel(x, y);
                if p[0] as u32 + p[1] as u32 + p[2] as u32 > 24 {
                    n += 1;
                }
            }
        }
        n
    };

    let y_bawah = belah as u32;
    let y_akhir = (kotak_teks.max_y().ceil() as u32).min(SISI as u32);

    // Control: without a clip, the bottom half really does contain text pixels.
    let mut tanpa_clip = Scene::new(Color::BLACK);
    tanpa_clip.push(run.clone());
    let img = target
        .render_with_glyphs(&gpu, &tanpa_clip, &mut mesin)
        .expect("render teks gagal");
    let bawah_utuh = hitung_baris(&img, y_bawah, y_akhir);
    let atas_utuh = hitung_baris(&img, 0, y_bawah);
    assert!(
        bawah_utuh > 50 && atas_utuh > 50,
        "kontrol tidak sahih: atas {atas_utuh}, bawah {bawah_utuh}"
    );

    // With the clip: the top rows survive, the bottom rows vanish completely.
    let mut dengan_clip = Scene::new(Color::BLACK);
    dengan_clip.push(Command::PushClip(atas));
    dengan_clip.push(run);
    dengan_clip.push(Command::PopClip);
    let img = target
        .render_with_glyphs(&gpu, &dengan_clip, &mut mesin)
        .expect("render teks gagal");

    assert_eq!(
        hitung_baris(&img, y_bawah, y_akhir),
        0,
        "separuh bawah teks bocor keluar clip"
    );
    let atas_terpotong = hitung_baris(&img, 0, y_bawah);
    assert_eq!(
        atas_terpotong, atas_utuh,
        "separuh atas teks ikut termakan clip"
    );
}

#[test]
fn setelah_pop_clip_gambar_berikutnya_tidak_terpotong() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    let clip = Rect::new(0.0, 0.0, 32.0, 32.0);
    let luar = Rect::new(64.0, 64.0, 56.0, 56.0);
    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(clip));
    scene.push(seluruh_kanvas(Color::WHITE));
    scene.push(Command::PopClip);
    scene.push(kotak(luar, Color::WHITE));

    let img = target.render(&gpu, &scene).expect("render gagal");

    assert_eq!(img.pixel(16, 16), PUTIH, "isi clip hilang");
    assert_eq!(img.pixel(40, 16), LATAR, "isi clip bocor");
    // The box after PopClip draws in full, far outside that earlier clip rect.
    assert_eq!(
        img.pixel(70, 70),
        PUTIH,
        "gambar setelah PopClip ikut terpotong"
    );
    assert_eq!(
        img.pixel(118, 118),
        PUTIH,
        "sisi jauh kotak kedua ikut terpotong"
    );
    assert_eq!(img.pixel(123, 123), LATAR);

    // The total area: the first box is exactly the clip's area (a hard scissor
    // edge), the second roughly its own (its SDF edge is anti-aliased, so its
    // four corners do not count as solid white).
    let terisi = jumlah(&img, PUTIH);
    assert!(
        (32 * 32 + 55 * 55..=32 * 32 + 56 * 56).contains(&terisi),
        "salah satu dari dua kotak tidak tergambar sebagaimana mestinya: {terisi}"
    );
}

#[test]
fn clip_bersarang_mengiris_dan_memulihkan_kotak_induk() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // `silka-core` sends rects that are ALREADY intersected: `dalam` below is
    // the result of outer ∩ inner viewport, not the inner viewport as-is.
    let luar = Rect::new(16.0, 16.0, 96.0, 96.0); // 16..112
    let dalam = Rect::new(48.0, 48.0, 32.0, 32.0); // 48..80

    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(luar));
    scene.push(seluruh_kanvas(Color::hex(0x0000FF)));
    scene.push(Command::PushClip(dalam));
    scene.push(seluruh_kanvas(Color::hex(0xFF0000)));
    scene.push(Command::PopClip);
    // A band lying entirely OUTSIDE the inner clip: if the clip stack were not
    // restored, this band would disappear completely.
    scene.push(kotak(
        Rect::new(0.0, 100.0, SISI, 28.0),
        Color::hex(0x00FF00),
    ));
    scene.push(Command::PopClip);

    let img = target.render(&gpu, &scene).expect("render gagal");

    // The inner clip's intersection: red covering exactly `dalam`.
    assert_eq!(img.pixel(64, 64), MERAH, "isi clip dalam hilang");
    assert_eq!(img.pixel(48, 48), MERAH);
    assert_eq!(img.pixel(47, 64), BIRU, "clip dalam bocor ke kiri");
    assert_eq!(img.pixel(80, 64), BIRU, "clip dalam bocor ke kanan");
    assert_eq!(jumlah(&img, MERAH), 32 * 32);

    // The outer clip still applies to everything outside the inner one.
    assert_eq!(img.pixel(20, 20), BIRU);
    assert_eq!(img.pixel(15, 20), LATAR, "clip luar bocor ke kiri");
    assert_eq!(img.pixel(112, 20), LATAR, "clip luar bocor ke kanan");
    assert_eq!(img.pixel(64, 15), LATAR, "clip luar bocor ke atas");

    // After the pop what applies is the PARENT rect — not the inner one, and
    // not "no clipping" either.
    assert_eq!(
        img.pixel(64, 105),
        HIJAU,
        "pita hilang: clip dalam masih aktif"
    );
    assert_eq!(img.pixel(15, 105), LATAR, "pita bocor keluar clip induk");
    assert_eq!(img.pixel(112, 105), LATAR, "pita bocor keluar clip induk");
    assert_eq!(img.pixel(64, 112), LATAR, "pita bocor ke bawah clip induk");
    assert_eq!(jumlah(&img, HIJAU), 96 * 12);

    // Blue = the whole outer clip minus the red and the green.
    assert_eq!(jumlah(&img, BIRU), 96 * 96 - 32 * 32 - 96 * 12);
}

#[test]
fn clip_di_luar_surface_tidak_menghasilkan_panik_atau_error_validasi() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // Entirely past the bottom-right of the surface, entirely past its
    // top-left, extremely far away, and extreme in size — all must be safe.
    for clip in [
        Rect::new(500.0, 500.0, 100.0, 100.0),
        Rect::new(-300.0, -300.0, 100.0, 100.0),
        Rect::new(SISI, 0.0, 10.0, 10.0),
        Rect::new(0.0, 0.0, 1.0e9, 1.0e9),
        Rect::new(-1.0e9, -1.0e9, 5.0e8, 5.0e8),
    ] {
        let mut scene = Scene::new(Color::BLACK);
        scene.push(Command::PushClip(clip));
        scene.push(seluruh_kanvas(Color::WHITE));
        scene.push(Command::PopClip);
        let img = target.render(&gpu, &scene).expect("render gagal");

        let terisi = jumlah(&img, PUTIH);
        if clip.min_x() <= 0.0 && clip.max_x() >= SISI {
            assert_eq!(terisi, (SISI * SISI) as usize, "clip raksasa: {clip:?}");
        } else {
            assert_eq!(terisi, 0, "clip di luar surface tetap menggambar: {clip:?}");
        }
    }
}

#[test]
fn clip_yang_setengah_keluar_surface_dijepit_bukan_dibuang() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // The clip's top-left corner lies outside the surface — clamped correctly,
    // what remains is 0..32, and there is no validation error.
    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(Rect::new(-32.0, -32.0, 64.0, 64.0)));
    scene.push(seluruh_kanvas(Color::WHITE));
    scene.push(Command::PopClip);

    let img = target.render(&gpu, &scene).expect("render gagal");
    assert_eq!(img.pixel(0, 0), PUTIH);
    assert_eq!(img.pixel(31, 31), PUTIH);
    assert_eq!(img.pixel(32, 31), LATAR);
    assert_eq!(jumlah(&img, PUTIH), 32 * 32);
}

#[test]
fn clip_kosong_tidak_menggambar_apa_pun() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // A viewport collapsed to zero (e.g. a scroll view with zero height).
    for clip in [
        Rect::new(32.0, 32.0, 0.0, 48.0),
        Rect::new(32.0, 32.0, 48.0, 0.0),
        Rect::new(32.0, 32.0, -48.0, -48.0),
    ] {
        let mut scene = Scene::new(Color::BLACK);
        scene.push(Command::PushClip(clip));
        scene.push(seluruh_kanvas(Color::WHITE));
        scene.push(Command::PopClip);
        let img = target.render(&gpu, &scene).expect("render gagal");
        assert_eq!(jumlah(&img, PUTIH), 0, "clip kosong: {clip:?}");
    }
}

#[test]
fn frame_berikutnya_tidak_mewarisi_clip_frame_sebelumnya() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // The same target is reused: the scissor is render pass state, so this is
    // what proves it does not bleed into the next frame.
    let mut terpotong = Scene::new(Color::BLACK);
    terpotong.push(Command::PushClip(Rect::new(0.0, 0.0, 16.0, 16.0)));
    terpotong.push(seluruh_kanvas(Color::WHITE));
    terpotong.push(Command::PopClip);
    let img = target.render(&gpu, &terpotong).expect("render gagal");
    assert_eq!(jumlah(&img, PUTIH), 16 * 16);

    let mut utuh = Scene::new(Color::BLACK);
    utuh.push(seluruh_kanvas(Color::WHITE));
    let img = target.render(&gpu, &utuh).expect("render gagal");
    assert_eq!(
        jumlah(&img, PUTIH),
        (SISI * SISI) as usize,
        "frame berikutnya masih terpotong oleh clip frame lalu"
    );
}
