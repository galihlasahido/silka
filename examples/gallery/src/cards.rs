//! Demo page: **squircle vs arc card grid**.
//!
//! One page for checking by eye what the unit tests already guard numerically
//! (REKOMENDASI §9.9: the gallery is a visual test tool, not a side example):
//!
//! - the left column uses **squircles** (superellipse, Apple-style continuous
//!   corners), the right column uses **arcs** (web-style circular arcs) with
//!   exactly the same nominal radius — the difference should read as a curve
//!   that "starts earlier" and a smoother transition into the straight edge,
//!   not as a rounder box;
//! - each row raises the radius (tokens `sm`→`xl`) and the elevation, so the
//!   **layered ambient + key shadows** get exercised too: the shadow must
//!   follow the card's corner shape;
//! - each card has a hairline border, to confirm the stroke sits exactly
//!   inside the edge of that same shape.
//!
//! This page is the **only place** allowed to pick a corner shape of its own,
//! because comparing the two is precisely its job. Every other value — colors,
//! radii, spacing, shadow recipes — still comes from the active theme tokens
//! (§2.6).
//!
//! ## Kept on purpose: this is the "before" picture
//!
//! Everything here is assembled by hand — the `Scene`, and the grid arithmetic
//! in [`kartu_kartu`] (`(width - padding * 2 - gap) * 0.5`, one multiplication
//! per cell). It predates both the widget layer and the utility vocabulary, and
//! it is deliberately **not** being rewritten: it is the reference the newer
//! path is measured against, and it stays reachable through `--page kartu`.
//!
//! The same picture through the framework is [`crate::reactive`]: no scene, no
//! arithmetic, no theme lookups in the styling, and hover/press/focus on top —
//! with the cards transitioning on springs instead of snapping. Read the two
//! files side by side; that comparison is the point.

use silka_paint::{CornerStyle, Corners, Quad, Rect, Scene, ShadowPair, Size};
use silka_theme::Theme;

/// How many cards per column (one row = one radius + one elevation).
const BARIS: usize = 4;

/// A single demo card, reduced to pure geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kartu {
    /// The card's rectangle in logical points.
    pub rect: Rect,
    /// The corner geometry being shown off.
    pub corners: Corners,
    /// The layered shadow recipe from the theme tokens.
    pub shadow: ShadowPair,
}

/// Assemble a single frame's scene for this page.
pub fn scene(theme: &Theme, size: Size) -> Scene {
    let mut scene = Scene::new(theme.color.background);
    // The hairline follows the spacing scale (0.25 steps = 1pt), not a loose
    // magic number.
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

/// Grid layout — pure logic, tested without a GPU.
///
/// Two columns (squircle on the left, arc on the right) × `BARIS` rows. If
/// the window is too narrow to fit the padding and gaps, the grid shrinks to
/// zero and never produces a card with a negative size.
pub fn kartu_kartu(theme: &Theme, size: Size) -> Vec<Kartu> {
    let padding = theme.space(6.0);
    let gap = theme.space(4.0);

    let lebar_kolom = ((size.width - padding * 2.0 - gap) * 0.5).max(0.0);
    let tinggi_baris =
        ((size.height - padding * 2.0 - gap * (BARIS as f32 - 1.0)) / BARIS as f32).max(0.0);

    // The radius grows per row; the elevation grows with it so the layered
    // shadows are seen developing alongside the shape.
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
                // The nominal radius is identical in both columns: what is
                // being compared is the shape, not the size.
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
            // The nominal radii must match exactly — otherwise the visual
            // comparison means nothing.
            assert_eq!(pasangan[0].corners.radii, pasangan[1].corners.radii);
            assert!(pasangan[0].rect.min_x() < pasangan[1].rect.min_x());
        }
    }

    #[test]
    fn perbandingan_tetap_berlaku_di_preset_tailwind() {
        // This page deliberately ignores `theme.radius.style` — in any preset
        // it must still show off both shapes.
        let t = Theme::tailwind(Appearance::Light);
        let k = kartu_kartu(&t, VIEWPORT);
        assert_eq!(k[0].corners.style, CornerStyle::squircle());
        assert_eq!(k[1].corners.style, CornerStyle::Arc);
        // …but the numbers still come from that preset's tokens
        // (Tailwind sm = 4pt).
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
        // Short window: the `xl` token must not exceed half the card height.
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
