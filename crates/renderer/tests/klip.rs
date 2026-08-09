//! Uji **piksel** untuk `Command::PushClip`/`PopClip`: apakah yang terpotong
//! benar-benar hilang di GPU.
//!
//! Sebelum milestone ini pasangan clip dilewati diam-diam oleh backend, dan
//! tidak ada satu pun test yang bisa memergokinya: pass paint sudah membuang
//! perintah yang **seluruhnya** di luar clip, jadi semua uji sisi CPU lulus
//! sementara konten yang terpotong **sebagian** tetap tergambar utuh di layar.
//! Itulah bentuk bug yang akan dilihat pengguna sebagai scroll view, list, dan
//! table yang bocor keluar viewport.
//!
//! Karena itu yang dibuktikan di sini adalah pikselnya, bukan strukturnya:
//!
//! 1. kotak yang jauh lebih besar dari clip hanya menyisakan piksel **di dalam**
//!    kotak clip — dan **nol** di luarnya, padahal geometri quad-nya melampaui;
//! 2. teks yang terbelah clip kehilangan separuh bawahnya, sementara kontrol
//!    tanpa clip menunjukkan separuh itu memang ada;
//! 3. setelah `PopClip` gambar berikutnya kembali tidak terpotong;
//! 4. clip bersarang menghasilkan irisan yang benar **dan** memulihkan kotak
//!    induk setelah pop;
//! 5. clip di luar surface tidak membuat panik maupun validation error wgpu.
//!
//! Tanpa adapter GPU, test dilewati dengan pesan — kegagalan palsu di CI jauh
//! lebih mahal daripada satu test yang absen.

use silka_paint::{Color, Command, Point, Quad, Rect, Scene, Size};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};
use silka_text::{TextConstraints, TextEngine, TextStyle};

/// Kanvas 128×128 poin — cukup lega untuk clip bersarang tanpa menyentuh tepi.
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

/// Kotak polos tanpa sudut membulat: tepinya lurus, jadi setiap piksel di
/// dalamnya terisi penuh dan bisa dihitung persis.
fn kotak(rect: Rect, warna: Color) -> Quad {
    Quad::new(rect).background(warna)
}

/// Kotak yang jauh lebih besar dari kanvas — geometrinya dijamin melampaui
/// clip apa pun yang dipakai test ini.
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

    // Clip 48×48 di tengah; kotaknya 128×128 — hampir tujuh kali lebih luas.
    let clip = Rect::new(32.0, 32.0, 48.0, 48.0);
    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(clip));
    scene.push(seluruh_kanvas(Color::WHITE));
    scene.push(Command::PopClip);

    let img = target.render(&gpu, &scene).expect("render gagal");

    // Di dalam: terisi penuh, termasuk piksel tepat di tepi clip.
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

    // Di luar: NOL, padahal quad-nya menutupi seluruh kanvas.
    assert_eq!(img.pixel(31, 56), LATAR, "bocor ke kiri clip");
    assert_eq!(img.pixel(80, 56), LATAR, "bocor ke kanan clip");
    assert_eq!(img.pixel(56, 31), LATAR, "bocor ke atas clip");
    assert_eq!(img.pixel(56, 80), LATAR, "bocor ke bawah clip");
    assert_eq!(img.pixel(0, 0), LATAR);
    assert_eq!(img.pixel(127, 127), LATAR);

    // Dan jumlahnya persis seluas clip — bukan seluas kanvas.
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

    // Clip yang sama dalam POIN LOGIS harus menutup empat kali lipat piksel
    // fisik di layar 2× — inilah jalur konversi `SurfaceGeometry`.
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

    // Belah tepat di tengah kotak teks, pada batas piksel bulat supaya
    // pembulatan-ke-luar scissor tidak mengaburkan yang sedang diuji.
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

    // Kontrol: tanpa clip, separuh bawah memang berisi piksel teks.
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

    // Dengan clip: baris atas tetap ada, baris bawah hilang sepenuhnya.
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
    // Kotak setelah PopClip tergambar utuh, jauh di luar kotak clip tadi.
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

    // Luas totalnya: kotak pertama persis seluas clip (tepi scissor keras),
    // kotak kedua kurang-lebih seluas dirinya (tepi SDF-nya anti-alias, jadi
    // empat sudutnya tidak dihitung sebagai putih pekat).
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

    // `silka-core` mengirim kotak yang SUDAH diiriskan: `dalam` di bawah ini
    // adalah hasil luar ∩ viewport dalam, bukan viewport dalam apa adanya.
    let luar = Rect::new(16.0, 16.0, 96.0, 96.0); // 16..112
    let dalam = Rect::new(48.0, 48.0, 32.0, 32.0); // 48..80

    let mut scene = Scene::new(Color::BLACK);
    scene.push(Command::PushClip(luar));
    scene.push(seluruh_kanvas(Color::hex(0x0000FF)));
    scene.push(Command::PushClip(dalam));
    scene.push(seluruh_kanvas(Color::hex(0xFF0000)));
    scene.push(Command::PopClip);
    // Pita yang seluruhnya di LUAR clip dalam: kalau tumpukan clip tidak
    // dipulihkan, pita ini akan hilang sama sekali.
    scene.push(kotak(
        Rect::new(0.0, 100.0, SISI, 28.0),
        Color::hex(0x00FF00),
    ));
    scene.push(Command::PopClip);

    let img = target.render(&gpu, &scene).expect("render gagal");

    // Irisan clip dalam: merah persis seluas `dalam`.
    assert_eq!(img.pixel(64, 64), MERAH, "isi clip dalam hilang");
    assert_eq!(img.pixel(48, 48), MERAH);
    assert_eq!(img.pixel(47, 64), BIRU, "clip dalam bocor ke kiri");
    assert_eq!(img.pixel(80, 64), BIRU, "clip dalam bocor ke kanan");
    assert_eq!(jumlah(&img, MERAH), 32 * 32);

    // Clip luar tetap berlaku untuk yang di luar clip dalam.
    assert_eq!(img.pixel(20, 20), BIRU);
    assert_eq!(img.pixel(15, 20), LATAR, "clip luar bocor ke kiri");
    assert_eq!(img.pixel(112, 20), LATAR, "clip luar bocor ke kanan");
    assert_eq!(img.pixel(64, 15), LATAR, "clip luar bocor ke atas");

    // Setelah pop, yang berlaku adalah kotak INDUK — bukan kotak dalam, dan
    // bukan pula "tanpa potong".
    assert_eq!(
        img.pixel(64, 105),
        HIJAU,
        "pita hilang: clip dalam masih aktif"
    );
    assert_eq!(img.pixel(15, 105), LATAR, "pita bocor keluar clip induk");
    assert_eq!(img.pixel(112, 105), LATAR, "pita bocor keluar clip induk");
    assert_eq!(img.pixel(64, 112), LATAR, "pita bocor ke bawah clip induk");
    assert_eq!(jumlah(&img, HIJAU), 96 * 12);

    // Biru = seluruh clip luar dikurangi merah dan hijau.
    assert_eq!(jumlah(&img, BIRU), 96 * 96 - 32 * 32 - 96 * 12);
}

#[test]
fn clip_di_luar_surface_tidak_menghasilkan_panik_atau_error_validasi() {
    let Some(gpu) = gpu() else { return };
    let mut target = kanvas(&gpu, 1.0);

    // Seluruhnya di kanan-bawah surface, seluruhnya di kiri-atas surface,
    // sangat jauh, dan berukuran ekstrem — semuanya harus aman.
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

    // Sudut kiri-atas clip berada di luar surface — kalau dijepit dengan benar
    // yang tersisa adalah 0..32, dan tidak ada validation error.
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

    // Viewport yang menyusut jadi nol (mis. scroll view dengan tinggi 0).
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

    // Target yang sama dipakai ulang: scissor adalah state render pass, jadi
    // ini yang membuktikan ia tidak menetes ke frame berikutnya.
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
