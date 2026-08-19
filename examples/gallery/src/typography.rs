//! Demo page: **typography specimen** (`glyph-atlas` milestone).
//!
//! What this page proves by eye — and what the unit tests guard numerically:
//!
//! - the **bundled Inter** really is being used, including its *variable font*
//!   nature: a weight run of 400 → 700 uses a single font file, not four
//!   (REKOMENDASI §3.6);
//! - **`measure(text, constraints)`** is what lays this page out: each block is
//!   placed below the previous one using the measured height, exactly as the
//!   box-constraints layout system will later do (§3.4);
//! - paragraph **wrapping** follows the column width, and `max_lines` truncates
//!   while flagging `overflowed` (the foundation for truncation/ellipsis);
//! - **font fallback**: a multi-script line must be readable, not tofu boxes —
//!   Inter carries no CJK/Arabic/emoji, so what shows up there comes from
//!   system fonts via cosmic-text (§3.3).
//!
//! All colors and spacing come from the active theme tokens; font sizes are
//! derived from the `typography.body_size` token, so the Cupertino (13pt) and
//! Tailwind (14pt) presets produce different scales on their own (§2.6).

use silka_paint::{Color, Point, Quad, Rect, Scene, Size};
use silka_text::{FontWeight, TextConstraints, TextEngine, TextStyle};
use silka_theme::Theme;

/// A block of text that has already been placed on the page.
#[derive(Debug, Clone)]
pub struct Blok {
    /// The text content.
    pub teks: String,
    /// Style (size, weight, tracking, wrapping).
    pub gaya: TextStyle,
    /// Color — always a semantic token.
    pub warna: Color,
    /// The block's top-left corner, in logical points.
    pub origin: Point,
    /// The measured height, in logical points.
    pub tinggi: f32,
}

/// Assemble a single frame's scene for this page.
pub fn scene(teks: &mut TextEngine, theme: &Theme, size: Size) -> Scene {
    let mut scene = Scene::new(theme.color.background);

    let panel = panel(theme, size);
    if !panel.size.is_empty() {
        scene.push_shadowed(
            Quad::new(panel)
                .background(theme.color.surface)
                .corners(theme.corners(theme.radius.lg).clamp_to(panel.size))
                .border(theme.space(0.25), theme.color.separator),
            theme.shadow.md,
        );
    }

    let batas = batas_kolom(theme, size);
    for blok in susun(teks, theme, size) {
        // Blocks stack top to bottom: once one block no longer fits in the
        // panel (the window got shorter), neither will any of the rest.
        if blok.origin.y + blok.tinggi > panel.max_y() {
            break;
        }
        let layout = teks.layout(&blok.teks, &blok.gaya, batas);
        let run = teks.rasterize(&layout, blok.origin, blok.warna);
        if !run.is_empty() {
            scene.push(run);
        }
    }

    scene
}

/// The panel rectangle that holds the specimen.
pub fn panel(theme: &Theme, size: Size) -> Rect {
    let margin = theme.space(6.0);
    Rect::new(
        margin,
        margin,
        (size.width - margin * 2.0).max(0.0),
        (size.height - margin * 2.0).max(0.0),
    )
}

/// Constraints for the text column inside the panel.
pub fn batas_kolom(theme: &Theme, size: Size) -> TextConstraints {
    let panel = panel(theme, size);
    TextConstraints::width((panel.size.width - theme.space(8.0)).max(0.0))
}

/// Page layout — **pure logic that can be tested without a GPU**.
///
/// This is a small demonstration of the "constraints go down, sizes come up"
/// protocol: each block is measured against the column width, then the vertical
/// cursor advances by the measured height plus a gap from the spacing scale.
pub fn susun(teks: &mut TextEngine, theme: &Theme, size: Size) -> Vec<Blok> {
    let panel = panel(theme, size);
    let batas = batas_kolom(theme, size);
    let kiri = panel.min_x() + theme.space(4.0);
    let mut y = panel.min_y() + theme.space(4.0);

    // The type scale is derived from tokens, not loose magic numbers: swapping
    // presets shifts the whole scale automatically.
    let body = theme.typography.body_size;
    let baris = theme.typography.body_line_height;

    let mut out: Vec<Blok> = Vec::new();
    let tambah = |out: &mut Vec<Blok>,
                  teks_engine: &mut TextEngine,
                  isi: &str,
                  gaya: TextStyle,
                  warna: Color,
                  jarak: f32,
                  y: &mut f32| {
        let ukuran = teks_engine.measure(isi, &gaya, batas);
        out.push(Blok {
            teks: isi.to_string(),
            gaya,
            warna,
            origin: Point::new(kiri, *y),
            tinggi: ukuran.height(),
        });
        *y += ukuran.height() + jarak;
    };

    tambah(
        &mut out,
        teks,
        "Typography",
        TextStyle::new()
            .size(body * 2.0)
            .weight(FontWeight::SEMIBOLD)
            // Negative tracking at large sizes — the SF habit that makes
            // headings feel "Apple" (§3.6).
            .tracking(-0.02)
            .line_height(1.15)
            .single_line(),
        theme.color.label,
        theme.space(1.0),
        &mut y,
    );

    tambah(
        &mut out,
        teks,
        "Inter variable, shaped by cosmic-text, rasterised into the glyph atlas.",
        TextStyle::new()
            .size(body * 1.15)
            .line_height(baris)
            .max_lines(2),
        theme.color.secondary_label,
        theme.space(4.0),
        &mut y,
    );

    tambah(
        &mut out,
        teks,
        "The greatest enemy of a new GUI framework is not rendering but \
         text: shaping, font fallback, bidi, and IME. That is why this layer \
         rides on cosmic-text, and all we write ourselves is the glyph atlas \
         with its subpixel-variant cache plus a measure API for the layout \
         system.",
        TextStyle::new().size(body).line_height(baris),
        theme.color.label,
        theme.space(4.0),
        &mut y,
    );

    for (nama, berat) in [
        ("Regular 400", FontWeight::REGULAR),
        ("Medium 500", FontWeight::MEDIUM),
        ("Semibold 600", FontWeight::SEMIBOLD),
        ("Bold 700", FontWeight::BOLD),
    ] {
        tambah(
            &mut out,
            teks,
            nama,
            TextStyle::new()
                .size(body * 1.3)
                .weight(berat)
                .line_height(baris)
                .single_line(),
            theme.color.label,
            theme.space(0.5),
            &mut y,
        );
    }

    tambah(
        &mut out,
        teks,
        "Fallback: 日本語 · 한국어 · العربية · Ελληνικά",
        TextStyle::new()
            .size(body * 1.15)
            .line_height(baris)
            .single_line(),
        theme.color.secondary_label,
        theme.space(2.0),
        &mut y,
    );

    tambah(
        &mut out,
        teks,
        "Tabular figures 0123456789 — and accents: àéîõü",
        TextStyle::new().size(body).line_height(baris).single_line(),
        theme.color.secondary_label,
        0.0,
        &mut y,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_paint::Command;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(1024.0, 720.0);

    /// A deterministic engine: with no system fonts, test results do not
    /// depend on whichever fonts happen to be installed on the CI machine
    /// (§9.5).
    fn mesin() -> TextEngine {
        TextEngine::bundled_only()
    }

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    #[test]
    fn blok_tersusun_dari_atas_ke_bawah_tanpa_bertumpuk() {
        let mut e = mesin();
        let blok = susun(&mut e, &tema(), VIEWPORT);
        assert!(blok.len() >= 8);
        for pasangan in blok.windows(2) {
            let bawah = pasangan[0].origin.y + pasangan[0].tinggi;
            assert!(
                pasangan[1].origin.y >= bawah - 1e-3,
                "blok bertumpuk: {:?} lalu {:?}",
                pasangan[0],
                pasangan[1]
            );
        }
    }

    #[test]
    fn semua_blok_berada_di_dalam_panel() {
        let mut e = mesin();
        let t = tema();
        let panel = panel(&t, VIEWPORT);
        for blok in susun(&mut e, &t, VIEWPORT) {
            assert!(blok.origin.x >= panel.min_x(), "{blok:?}");
            assert!(blok.origin.y >= panel.min_y(), "{blok:?}");
            assert!(blok.origin.y + blok.tinggi <= panel.max_y(), "{blok:?}");
        }
    }

    #[test]
    fn judul_lebih_besar_dari_body() {
        let mut e = mesin();
        let t = tema();
        let blok = susun(&mut e, &t, VIEWPORT);
        assert_eq!(blok[0].teks, "Typography");
        assert!(blok[0].gaya.size > t.typography.body_size);
        assert!(blok[0].tinggi > blok[1].tinggi / 2.0);
    }

    #[test]
    fn warna_teks_selalu_token_semantik() {
        let mut e = mesin();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                for blok in susun(&mut e, &t, VIEWPORT) {
                    assert!(
                        blok.warna == t.color.label || blok.warna == t.color.secondary_label,
                        "warna lepas dari token: {blok:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn skala_tipografi_mengikuti_preset() {
        let mut e = mesin();
        let cupertino = susun(&mut e, &Theme::cupertino(Appearance::Light), VIEWPORT);
        let tailwind = susun(&mut e, &Theme::tailwind(Appearance::Light), VIEWPORT);
        // Tailwind body 14pt > Cupertino 13pt, so its heading is bigger too.
        assert!(tailwind[0].gaya.size > cupertino[0].gaya.size);
    }

    #[test]
    fn paragraf_membungkus_mengikuti_lebar_kolom() {
        let mut e = mesin();
        let t = tema();
        let paragraf = susun(&mut e, &t, VIEWPORT)
            .into_iter()
            .find(|b| b.teks.starts_with("The greatest enemy"))
            .expect("paragraf ada");

        let lebar = e.measure(&paragraf.teks, &paragraf.gaya, batas_kolom(&t, VIEWPORT));
        let sempit = e.measure(
            &paragraf.teks,
            &paragraf.gaya,
            batas_kolom(&t, Size::new(520.0, 720.0)),
        );
        assert!(lebar.line_count >= 2);
        assert!(
            sempit.line_count > lebar.line_count,
            "kolom lebih sempit harus lebih banyak baris: {lebar:?} vs {sempit:?}"
        );
    }

    #[test]
    fn subjudul_dibatasi_dua_baris() {
        let mut e = mesin();
        let t = tema();
        let sub = &susun(&mut e, &t, VIEWPORT)[1];
        assert_eq!(sub.gaya.max_lines, Some(2));
        // A column this narrow needs more than two lines — so the remaining
        // lines really are truncated, rather than happening to fit.
        let sempit = batas_kolom(&t, Size::new(220.0, 720.0));
        let mut tanpa_batas = sub.gaya.clone();
        tanpa_batas.max_lines = None;
        assert!(e.measure(&sub.teks, &tanpa_batas, sempit).line_count > 2);

        let m = e.measure(&sub.teks, &sub.gaya, sempit);
        assert_eq!(m.line_count, 2);
        assert!(m.overflowed, "sisa baris dipotong harus ditandai overflow");
    }

    #[test]
    fn skala_berat_memakai_satu_variable_font() {
        let mut e = mesin();
        let t = tema();
        let berat: Vec<FontWeight> = susun(&mut e, &t, VIEWPORT)
            .iter()
            .filter(|b| {
                b.teks.ends_with("400")
                    || b.teks.ends_with("500")
                    || b.teks.ends_with("600")
                    || b.teks.ends_with("700")
            })
            .map(|b| b.gaya.weight)
            .collect();
        assert_eq!(
            berat,
            vec![
                FontWeight::REGULAR,
                FontWeight::MEDIUM,
                FontWeight::SEMIBOLD,
                FontWeight::BOLD
            ]
        );
        // A single font file for all of them.
        assert!(e.ui_family().is_some_and(|f| f.contains("Inter")));
    }

    #[test]
    fn scene_berisi_panel_dan_glyph_run() {
        let mut e = mesin();
        let t = tema();
        let s = scene(&mut e, &t, VIEWPORT);
        assert_eq!(s.clear_color(), t.color.background);

        let mut kotak = 0;
        let mut glyph = 0;
        let mut total_glyph = 0;
        for c in s.commands() {
            match c {
                Command::Quad(q) => {
                    kotak += 1;
                    assert_eq!(q.background, t.color.surface);
                }
                Command::GlyphRun(r) => {
                    glyph += 1;
                    total_glyph += r.len();
                    assert!(r.color == t.color.label || r.color == t.color.secondary_label);
                }
                Command::Shadow(_) => {}
                lain => panic!("perintah tak terduga: {lain:?}"),
            }
        }
        assert_eq!(kotak, 1, "satu panel");
        assert!(glyph >= 8, "satu glyph run per blok teks: {glyph}");
        assert!(
            total_glyph > 100,
            "spesimen harus padat teks: {total_glyph}"
        );
        assert!(!e.glyphs().is_empty(), "atlas terisi");
    }

    #[test]
    fn glyph_run_berada_di_dalam_panel() {
        let mut e = mesin();
        let t = tema();
        let panel = panel(&t, VIEWPORT);
        for c in scene(&mut e, &t, VIEWPORT).commands() {
            if let Command::GlyphRun(r) = c {
                let b = r.bounds().expect("run tidak kosong");
                assert!(b.min_x() >= panel.min_x() - 2.0, "{b:?}");
                assert!(b.max_x() <= panel.max_x() + 2.0, "{b:?}");
                assert!(b.max_y() <= panel.max_y() + 2.0, "{b:?}");
            }
        }
    }

    #[test]
    fn frame_kedua_tidak_menambah_glyph_baru() {
        // Proof the atlas really is a cache: drawing the same frame twice
        // rasterizes nothing more (§3.5 "render only when dirty" only means
        // something if repeating a frame really is cheap).
        let mut e = mesin();
        let t = tema();
        scene(&mut e, &t, VIEWPORT);
        let sesudah_frame_1 = e.glyphs().len();
        scene(&mut e, &t, VIEWPORT);
        assert_eq!(e.glyphs().len(), sesudah_frame_1);
    }

    /// This page is rendered to an offscreen texture through **exactly the
    /// same** path as the window (pipeline, sRGB format, blending, glyph
    /// atlas), then its pixels are counted. This is what catches "the text
    /// never reached the GPU": every other test in this file stops on the CPU
    /// side and stays green even with a blank screen.
    #[test]
    fn spesimen_teks_benar_benar_tergambar_di_gpu() {
        use silka_paint::Command;
        use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};

        const SKALA: f64 = 2.0;

        let Ok(gpu) = Gpu::headless() else {
            eprintln!("dilewati: tidak ada GPU untuk render headless");
            return;
        };
        let mut e = mesin();
        e.set_scale_factor(SKALA as f32);
        let t = tema();
        let mut target = OffscreenTarget::new(&gpu, SurfaceGeometry::from_logical(VIEWPORT, SKALA))
            .expect("target headless");

        // Sample area: the panel's interior, far enough from the border that
        // anti-aliased edges are not counted.
        let panel = panel(&t, VIEWPORT).deflate(silka_paint::Insets::all(2.0));
        let permukaan = t.color.surface;
        let hitung = |img: &Rgba8Image| {
            let f = |v: f32| (v as f64 * SKALA).round().max(0.0) as u32;
            let mut n = 0;
            for y in f(panel.min_y())..f(panel.max_y()).min(img.height()) {
                for x in f(panel.min_x())..f(panel.max_x()).min(img.width()) {
                    let p = img.pixel(x, y);
                    let jauh = |c: u8, token: f32| (c as f32 - token * 255.0).abs() > 30.0;
                    if jauh(p[0], permukaan.r) || jauh(p[1], permukaan.g) || jauh(p[2], permukaan.b)
                    {
                        n += 1;
                    }
                }
            }
            n
        };

        let scene = scene(&mut e, &t, VIEWPORT);
        let jumlah_run = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::GlyphRun(_)))
            .count();
        assert!(jumlah_run >= 8);

        let dengan_teks = target
            .render_with_glyphs(&gpu, &scene, &mut e)
            .expect("render halaman teks");
        let terisi = hitung(&dengan_teks);
        assert!(
            terisi > 5_000,
            "halaman teks nyaris kosong di layar: hanya {terisi} piksel bukan-permukaan"
        );

        // Negative control: the same panel with no GlyphRun at all must yield
        // ZERO non-surface pixels over the same area.
        let mut tanpa_teks = Scene::new(t.color.background);
        let kotak = super::panel(&t, VIEWPORT);
        tanpa_teks.push_shadowed(
            Quad::new(kotak)
                .background(t.color.surface)
                .corners(t.corners(t.radius.lg).clamp_to(kotak.size))
                .border(t.space(0.25), t.color.separator),
            t.shadow.md,
        );
        let polos = target
            .render_with_glyphs(&gpu, &tanpa_teks, &mut e)
            .expect("render panel polos");
        assert_eq!(
            hitung(&polos),
            0,
            "panel polos sudah punya piksel bukan-permukaan — ambang sampelnya salah"
        );
    }

    #[test]
    fn window_terlalu_kecil_tidak_membuat_ukuran_negatif() {
        let mut e = mesin();
        let t = tema();
        for size in [Size::ZERO, Size::new(10.0, 10.0), Size::new(0.0, 720.0)] {
            let p = panel(&t, size);
            assert!(p.size.width >= 0.0 && p.size.height >= 0.0, "{size:?}");
            assert!(batas_kolom(&t, size).max_width >= 0.0, "{size:?}");
            // Must not panic.
            let _ = scene(&mut e, &t, size);
        }
    }
}
