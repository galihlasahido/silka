//! Halaman demo: **grid kartu squircle vs arc**.
//!
//! Satu halaman untuk memeriksa dengan mata apa yang sudah dijaga unit test
//! secara angka (REKOMENDASI §9.9: gallery adalah alat uji visual, bukan
//! contoh sampingan):
//!
//! - kolom kiri memakai **squircle** (superellipse, continuous corner ala
//!   Apple), kolom kanan memakai **arc** (busur lingkaran ala web) dengan
//!   radius nominal yang persis sama — perbedaannya harus terlihat sebagai
//!   lengkung yang "mulai lebih awal" dan transisi yang lebih halus ke sisi
//!   lurus, bukan sebagai kotak yang lebih bulat;
//! - setiap baris menaikkan radius (token `sm`→`xl`) dan elevasi, sehingga
//!   **bayangan ganda ambient + key** ikut teruji: bayangan harus mengikuti
//!   bentuk sudut kartunya;
//! - setiap kartu punya border hairline, untuk memastikan stroke berada tepat
//!   di dalam tepi bentuk yang sama.
//!
//! Halaman ini adalah **satu-satunya tempat** yang boleh memilih bentuk sudut
//! sendiri, karena tugasnya justru membandingkan keduanya. Semua nilai
//! lainnya — warna, radius, spacing, resep bayangan — tetap datang dari token
//! theme aktif (§2.6).

use silka_paint::{CornerStyle, Corners, Quad, Rect, Scene, ShadowPair, Size};
use silka_theme::Theme;

/// Berapa kartu per kolom (satu baris = satu radius + satu elevasi).
const BARIS: usize = 4;

/// Satu kartu demo, sudah jadi geometri murni.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kartu {
    /// Kotak kartu dalam poin logis.
    pub rect: Rect,
    /// Geometri sudut yang sedang dipamerkan.
    pub corners: Corners,
    /// Resep bayangan ganda dari token theme.
    pub shadow: ShadowPair,
}

/// Susun scene satu frame untuk halaman ini.
pub fn scene(theme: &Theme, size: Size) -> Scene {
    let mut scene = Scene::new(theme.color.background);
    // Hairline mengikuti skala spacing (0.25 langkah = 1pt), bukan angka lepas.
    let border = theme.space(0.25);
    for kartu in kartu_kartu(theme, size) {
        scene.push_shadowed(
            Quad::new(kartu.rect)
                .background(theme.color.surface)
                .corners(kartu.corners)
                .border(border, theme.color.separator),
            kartu.shadow,
        );
    }
    scene
}

/// Tata letak grid — logika murni, diuji tanpa GPU.
///
/// Dua kolom (squircle di kiri, arc di kanan) × [`BARIS`] baris. Bila window
/// terlalu sempit untuk menampung padding dan gap, grid mengecil sampai nol
/// dan tidak pernah menghasilkan kartu berukuran negatif.
pub fn kartu_kartu(theme: &Theme, size: Size) -> Vec<Kartu> {
    let padding = theme.space(6.0);
    let gap = theme.space(4.0);

    let lebar_kolom = ((size.width - padding * 2.0 - gap) * 0.5).max(0.0);
    let tinggi_baris =
        ((size.height - padding * 2.0 - gap * (BARIS as f32 - 1.0)) / BARIS as f32).max(0.0);

    // Radius naik per baris; elevasi ikut naik supaya bayangan ganda terlihat
    // berkembang bersama bentuknya.
    let baris = [
        (theme.radius.sm, theme.shadow.sm),
        (theme.radius.md, theme.shadow.sm),
        (theme.radius.lg, theme.shadow.md),
        (theme.radius.xl, theme.shadow.lg),
    ];
    let kolom = [CornerStyle::squircle(), CornerStyle::Arc];

    let mut out = Vec::with_capacity(BARIS * kolom.len());
    for (i, (radius, shadow)) in baris.into_iter().enumerate() {
        let y = padding + (tinggi_baris + gap) * i as f32;
        for (j, style) in kolom.into_iter().enumerate() {
            let x = padding + (lebar_kolom + gap) * j as f32;
            let rect = Rect::new(x, y, lebar_kolom, tinggi_baris);
            out.push(Kartu {
                rect,
                // Radius nominal identik di kedua kolom: yang dibandingkan
                // adalah bentuknya, bukan besarnya.
                corners: Corners::uniform(radius, style).clamp_to(rect.size),
                shadow,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_paint::Command;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(1024.0, 720.0);

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    #[test]
    fn grid_dua_kolom_empat_baris() {
        assert_eq!(kartu_kartu(&tema(), VIEWPORT).len(), BARIS * 2);
    }

    #[test]
    fn kolom_kiri_squircle_kolom_kanan_arc() {
        for pasangan in kartu_kartu(&tema(), VIEWPORT).chunks(2) {
            assert_eq!(pasangan[0].corners.style, CornerStyle::squircle());
            assert_eq!(pasangan[1].corners.style, CornerStyle::Arc);
            // Radius nominalnya harus sama persis — kalau tidak, perbandingan
            // visualnya tidak berarti apa-apa.
            assert_eq!(pasangan[0].corners.radii, pasangan[1].corners.radii);
            assert!(pasangan[0].rect.min_x() < pasangan[1].rect.min_x());
        }
    }

    #[test]
    fn perbandingan_tetap_berlaku_di_preset_tailwind() {
        // Halaman ini sengaja mengabaikan `theme.radius.style` — di preset
        // mana pun ia harus tetap memamerkan kedua bentuk.
        let t = Theme::tailwind(Appearance::Light);
        let k = kartu_kartu(&t, VIEWPORT);
        assert_eq!(k[0].corners.style, CornerStyle::squircle());
        assert_eq!(k[1].corners.style, CornerStyle::Arc);
        // …tapi angkanya tetap dari token preset itu (Tailwind sm = 4pt).
        assert_eq!(k[0].corners.radii.max(), t.radius.sm);
    }

    #[test]
    fn radius_naik_setiap_baris() {
        let t = tema();
        let k = kartu_kartu(&t, VIEWPORT);
        let radius: Vec<f32> = k.chunks(2).map(|b| b[0].corners.radii.max()).collect();
        assert_eq!(
            radius,
            vec![t.radius.sm, t.radius.md, t.radius.lg, t.radius.xl]
        );
        assert!(radius.windows(2).all(|w| w[0] < w[1]), "{radius:?}");
    }

    #[test]
    fn elevasi_naik_sampai_baris_terakhir() {
        let t = tema();
        let k = kartu_kartu(&t, VIEWPORT);
        let pertama = k[0].shadow.ambient.blur;
        let terakhir = k[k.len() - 1].shadow.ambient.blur;
        assert!(terakhir > pertama, "{pertama} → {terakhir}");
    }

    #[test]
    fn semua_kartu_berada_di_dalam_viewport() {
        for kartu in kartu_kartu(&tema(), VIEWPORT) {
            assert!(
                kartu.rect.min_x() >= 0.0 && kartu.rect.min_y() >= 0.0,
                "{kartu:?}"
            );
            assert!(kartu.rect.max_x() <= VIEWPORT.width + 1e-3, "{kartu:?}");
            assert!(kartu.rect.max_y() <= VIEWPORT.height + 1e-3, "{kartu:?}");
            assert!(!kartu.rect.size.is_empty(), "{kartu:?}");
        }
    }

    #[test]
    fn kartu_tidak_saling_menimpa() {
        let k = kartu_kartu(&tema(), VIEWPORT);
        for pasangan in k.chunks(2) {
            assert!(pasangan[0].rect.max_x() <= pasangan[1].rect.min_x());
        }
        for baris in k.chunks(2).collect::<Vec<_>>().windows(2) {
            assert!(baris[0][0].rect.max_y() <= baris[1][0].rect.min_y());
        }
    }

    #[test]
    fn window_terlalu_kecil_tidak_membuat_ukuran_negatif() {
        for size in [Size::ZERO, Size::new(10.0, 10.0), Size::new(0.0, 720.0)] {
            for kartu in kartu_kartu(&tema(), size) {
                assert!(kartu.rect.size.width >= 0.0, "{size:?}");
                assert!(kartu.rect.size.height >= 0.0, "{size:?}");
            }
        }
    }

    #[test]
    fn radius_dibatasi_terhadap_kartu_yang_gepeng() {
        // Window pendek: token `xl` tidak boleh melebihi separuh tinggi kartu.
        let sempit = Size::new(400.0, 200.0);
        for kartu in kartu_kartu(&tema(), sempit) {
            let batas = kartu.rect.size.min_side() * 0.5;
            assert!(kartu.corners.radii.max() <= batas + 1e-3, "{kartu:?}");
        }
    }

    #[test]
    fn setiap_kartu_menjadi_dua_bayangan_plus_satu_kotak() {
        let s = scene(&tema(), VIEWPORT);
        assert_eq!(s.len(), BARIS * 2 * 3);
        match s.commands() {
            [Command::Shadow(_), Command::Shadow(_), Command::Quad(_), ..] => {}
            lain => panic!("urutan perintah salah: {:?}", &lain[..3.min(lain.len())]),
        }
    }

    #[test]
    fn warna_selalu_datang_dari_token() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = scene(&t, VIEWPORT);
                assert_eq!(s.clear_color(), t.color.background);
                let kotak: Vec<_> = s
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q),
                        _ => None,
                    })
                    .collect();
                assert!(!kotak.is_empty());
                for q in kotak {
                    assert_eq!(q.background, t.color.surface, "{preset:?} {appearance:?}");
                    assert_eq!(q.border_color, t.color.separator);
                    assert_eq!(q.border_width, 1.0, "hairline = 0.25 langkah spacing");
                }
            }
        }
    }
}
