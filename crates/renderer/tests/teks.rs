//! Uji rasterisasi **teks** headless: apakah glyph benar-benar sampai ke GPU.
//!
//! Ini tambalan untuk lubang paling mahal di jalur render: sebelumnya
//! `Command::GlyphRun` dibuang diam-diam oleh backend, dan tidak ada satu pun
//! test yang bisa memergokinya — semua uji teks berhenti di sisi CPU (atlas
//! terisi, `GlyphRun` terbentuk) dan semua uji GPU hanya menggambar kotak.
//! Karena itu yang dibuktikan di sini adalah **pikselnya**:
//!
//! 1. scene berisi teks menghasilkan piksel teks di dalam kotak teks;
//! 2. scene yang sama **tanpa** `GlyphRun` menghasilkan **nol** piksel di
//!    kotak itu (kontrol negatif — tanpa ini, angka apa pun bisa datang dari
//!    latar atau panel);
//! 3. scene berisi teks yang dirender **tanpa sumber atlas** juga nol —
//!    membuktikan piksel itu memang datang dari atlas, bukan dari kebetulan;
//! 4. teks digambar **di atas** kotak yang mendahuluinya, tidak tertimpa;
//! 5. warnanya datang dari perintah gambar (token theme), bukan dari atlas;
//! 6. di layar 2× glyph benar-benar dirasterisasi pada resolusi layar.
//!
//! Semuanya memakai font **bundel** (`TextEngine::bundled_only`) supaya hasil
//! di CI tidak tergantung font yang kebetulan terpasang (REKOMENDASI §9.5).
//!
//! Tanpa adapter GPU, test dilewati dengan pesan — kegagalan palsu di CI jauh
//! lebih mahal daripada satu test yang absen.

use silka_paint::{Color, Point, Quad, Rect, Scene, Size};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};
use silka_text::{TextConstraints, TextEngine, TextStyle};

/// Kanvas 320×120 poin — muat satu baris teks besar dengan margin lega.
const LEBAR: f32 = 320.0;
const TINGGI: f32 = 120.0;
/// Sudut kiri-atas blok teks.
const ORIGIN: Point = Point::new(16.0, 24.0);
/// Ukuran font uji: cukup besar agar jumlah piksel jauh di atas derau.
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

/// Scene berisi satu baris teks, beserta kotak logis yang ditempatinya.
fn scene_teks(mesin: &mut TextEngine, warna: Color) -> (Scene, Rect) {
    let mut scene = Scene::new(Color::BLACK);
    let layout = mesin.layout(CONTOH, &gaya(), TextConstraints::UNBOUNDED);
    let run = mesin.rasterize(&layout, ORIGIN, warna);
    let kotak = run.bounds().expect("teks harus punya glyph");
    scene.push(run);
    (scene, kotak)
}

/// Piksel dalam kotak logis yang bukan latar (latar = hitam pekat).
///
/// Ambang 24 memberi ruang untuk tepi anti-alias yang sangat samar, tapi jauh
/// di bawah cakupan glyph yang sesungguhnya.
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
    // Teks bukan blok pejal: sebagian besar kotaknya tetap latar.
    assert!(
        terisi < luas * 3 / 4,
        "kotak teks malah tergambar penuh ({terisi} dari {luas}) — bukan glyph"
    );
    // Di luar kotak teks tidak boleh ada apa pun.
    assert_eq!(img.pixel(1, 1), [0, 0, 0, 255], "latar ikut ternoda");
}

#[test]
fn scene_tanpa_glyph_run_menghasilkan_nol_piksel_teks() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // Kotak diambil dari scene yang BERISI teks, lalu diukur pada scene yang
    // tidak berisi apa-apa — persis area yang sama, hasilnya harus nol.
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
    // `render` tanpa sumber atlas: perintah glyph ada, bitmapnya tidak.
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

    // Panel putih pejal dulu, teks gelap sesudahnya — kalau urutan gambar
    // rusak, teks akan tertimpa dan kotak ini akan putih rata.
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
    // Di luar kotak teks panel tetap putih bersih.
    assert_eq!(img.pixel(1, 1), [255, 255, 255, 255]);
}

#[test]
fn warna_teks_datang_dari_token_bukan_dari_atlas() {
    let Some(gpu) = gpu() else { return };
    let mut mesin = mesin(1.0);
    let mut target = kanvas(&gpu, 1.0);

    // Dua warna, satu atlas: bitmap yang sama harus melayani keduanya.
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
    // Kotak logisnya nyaris sama — yang berlipat ganda adalah pikselnya.
    assert!((kotak_2x.size.width - kotak.size.width).abs() < 2.0);

    let n1 = piksel_teks(&img_1x, kotak, 1.0);
    let n2 = piksel_teks(&img_2x, kotak_2x, 2.0);
    assert!(
        n2 > n1 * 2,
        "teks 2x tidak lebih rinci: {n1} px pada 1x, {n2} px pada 2x"
    );

    // Tajam, bukan lembek: glyph pada 2x tetap punya inti yang pekat penuh.
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

    // Setelah frame pertama tidak ada lagi yang kotor: kalau frame kedua tetap
    // benar, berarti tekstur memang bertahan dan tidak diunggah ulang.
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

    // Frame pertama mengisi atlas dengan satu himpunan glyph…
    let (scene_a, kotak_a) = scene_teks(&mut mesin, Color::WHITE);
    let img_a = target
        .render_with_glyphs(&gpu, &scene_a, &mut mesin)
        .expect("frame A gagal");
    assert!(piksel_teks(&img_a, kotak_a, 1.0) > 200);

    // …frame kedua memakai huruf yang belum pernah ada. Kalau unggahan
    // inkremental salah kotak, teks baru ini akan kosong.
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

/// Atlas buatan yang bisa dipaksa berganti ukuran — jalur "atlas penuh lalu
/// dibangun ulang" terlalu jarang terjadi dengan font sungguhan untuk
/// diandalkan sebagai uji, padahal justru di situ tekstur GPU harus dibuat
/// ulang **dan** bind group dirakit ulang.
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

    // Atlas kecil dulu…
    let mut atlas = AtlasBuatan::baru(16, AtlasRegion::new(0, 0, 8, 8));
    let kecil = target
        .render_with_glyphs(&gpu, &scene, &mut atlas)
        .expect("render atlas kecil gagal");
    assert_eq!(kecil.pixel(12, 12), [255, 255, 255, 255]);

    // …lalu atlas dibangun ulang lebih besar, dengan glyph di tempat lain.
    // Kalau tekstur/bind group tidak ikut diperbarui, kotak ini jadi hitam.
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

    // Banyak ukuran font berbeda = banyak bitmap = atlas dipaksa tumbuh.
    // Yang diuji: setelah tekstur dibuat ulang, bind group ikut dirakit ulang
    // dan isinya diunggah penuh — kalau tidak, teks lenyap tanpa error.
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
