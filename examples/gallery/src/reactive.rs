//! Demo page: **the card grid drawn through the reactive lifecycle**.
//!
//! Its visual content is deliberately identical to the [`crate::cards`] page —
//! squircles on the left, arcs on the right, radius and elevation growing per
//! row — but the route there differs, and that is the point of this page:
//! **not a single `Scene` is assembled by hand here**. All that is written is
//! the view tree; the scene is born from `signals → view-diff → layout → paint`
//! inside [`silka_core::app::AppRuntime`] (REKOMENDASI §2, §3.5).
//!
//! Running this page also proves two things:
//!
//! 1. **Layout is computed by the engine**, not by arithmetic in the page code.
//!    There is no `padding * 2.0 - gap` in this file; positions come from
//!    `column`/`row` and `expanded()` (§3.4).
//! 2. **The theme is a signal.** A change in OS dark mode writes to
//!    `Signal<Theme>`, and only the components that actually read it are
//!    rebuilt (§2.7).

use silka_core::app::{component, BuildCtx};
use silka_core::signals::{Key, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, fixed, row, View};
use silka_paint::{CornerStyle, Corners, Insets, ShadowPair};
use silka_theme::Theme;

/// How many card rows (one row = one radius + one elevation).
pub const BARIS: usize = 4;

/// The view tree for the whole page — this is what gets handed to `run_app`.
///
/// Read in the root scope: a theme change rebuilds this page in its entirety,
/// which is exactly what we want since every color here is a token.
pub fn halaman(cx: &BuildCtx) -> View {
    let theme: Signal<Theme> = cx.expect_env();
    let t = theme.get();
    let gap = t.space(4.0);

    column(
        (0..BARIS)
            .map(|baris| {
                View::from(expanded(
                    row([expanded(kartu(baris, 0)), expanded(kartu(baris, 1))])
                        .spacing(gap)
                        // Cards are as tall as their row, not as tall as
                        // their content.
                        .cross(CrossAlign::Stretch),
                ))
            })
            .collect::<Vec<View>>(),
    )
    .spacing(gap)
    // Each row spans the page width; without this the `expanded()` inside it
    // has no space to divide up.
    .cross(CrossAlign::Stretch)
    .padding(Insets::all(t.space(6.0)))
    .into()
}

/// A single card as its own component.
///
/// Each card gets its own scope, so once a card gains state (hover, pressed)
/// only that card is rebuilt — not the whole grid.
fn kartu(baris: usize, kolom: usize) -> View {
    component(Key::num((baris * 2 + kolom) as i64), move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let (radius, shadow) = gaya_baris(&t, baris);
        // Squircle on the left, arc on the right — this page is the only one
        // allowed to pick its own corner shape, since comparing them is its
        // job.
        let style = if kolom == 0 {
            CornerStyle::squircle()
        } else {
            CornerStyle::Arc
        };
        // Zero size: the `expanded()` above hands down tight constraints, so
        // the card fills its cell. The layout numbers belong to the layout
        // engine.
        fixed(0.0, 0.0)
            .background(t.color.surface)
            .corners(Corners::uniform(radius, style))
            .border(t.space(0.25), t.color.separator)
            .shadow(shadow)
            .into()
    })
}

/// Radius + elevation for a row — both tokens, not loose magic numbers.
fn gaya_baris(t: &Theme, baris: usize) -> (f32, ShadowPair) {
    match baris {
        0 => (t.radius.sm, t.shadow.sm),
        1 => (t.radius.md, t.shadow.sm),
        2 => (t.radius.lg, t.shadow.md),
        _ => (t.radius.xl, t.shadow.lg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::{app, AppRuntime};
    use silka_paint::{Command, Quad, Size};
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(1024.0, 720.0);

    /// A headless app with the same theme injection `run_app` performs.
    fn ui(theme: Theme) -> AppRuntime {
        app(halaman)
            .with_env(move |rt| rt.signal(theme))
            .clear_color(theme.color.background)
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    fn kotak(ui: &AppRuntime) -> Vec<Quad> {
        ui.scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn satu_kartu_per_sel_dan_semuanya_bertumpu_pada_bayangan_ganda() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        assert_eq!(kotak(&ui).len(), BARIS * 2);
        // Two shadows + one quad per card, just like the `kartu` page.
        assert_eq!(ui.scene().len(), BARIS * 2 * 3);
    }

    #[test]
    fn tata_letak_dihitung_mesin_bukan_oleh_halaman() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        let k = kotak(&ui);
        for baris in k.chunks(2) {
            // Left and right are equally wide, aligned, and never overlap.
            assert_eq!(baris[0].rect.size, baris[1].rect.size);
            assert_eq!(baris[0].rect.min_y(), baris[1].rect.min_y());
            assert!(baris[0].rect.max_x() <= baris[1].rect.min_x() + 1e-3);
        }
        for dua in k.chunks(2).collect::<Vec<_>>().windows(2) {
            assert!(dua[0][0].rect.max_y() <= dua[1][0].rect.min_y() + 1e-3);
        }
        // Everything is inside the viewport, and nothing has zero size.
        for q in &k {
            assert!(q.rect.min_x() >= 0.0 && q.rect.min_y() >= 0.0, "{q:?}");
            assert!(q.rect.max_x() <= VIEWPORT.width + 1e-3, "{q:?}");
            assert!(q.rect.max_y() <= VIEWPORT.height + 1e-3, "{q:?}");
            assert!(!q.rect.size.is_empty(), "{q:?}");
        }
    }

    #[test]
    fn kolom_kiri_squircle_kolom_kanan_arc() {
        let mut ui = ui(Theme::tailwind(Appearance::Light));
        ui.frame();
        for baris in kotak(&ui).chunks(2) {
            assert_eq!(baris[0].corners.style, CornerStyle::squircle());
            assert_eq!(baris[1].corners.style, CornerStyle::Arc);
            assert_eq!(baris[0].corners.radii, baris[1].corners.radii);
        }
    }

    #[test]
    fn warna_selalu_datang_dari_token() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);
                for q in kotak(&ui) {
                    assert_eq!(q.background, t.color.surface, "{preset:?} {appearance:?}");
                    assert_eq!(q.border_color, t.color.separator);
                    assert_eq!(q.border_width, t.space(0.25));
                }
            }
        }
    }

    #[test]
    fn ganti_theme_membangun_ulang_halaman_dan_idle_lagi_sesudahnya() {
        let terang = Theme::cupertino(Appearance::Light);
        let mut ui = ui(terang);
        ui.frame();
        assert!(ui.is_idle(), "halaman statis tidak menyisakan pekerjaan");

        let gelap = Theme::cupertino(Appearance::Dark);
        let signal: Signal<Theme> = ui.env().expect("theme dititipkan di Env");
        signal.set(gelap);
        assert!(!ui.is_idle(), "theme berubah menjadwalkan tepat satu frame");

        ui.set_clear_color(gelap.color.background);
        let laporan = ui.frame();
        assert_eq!(laporan.rebuilt, 1, "akar yang membaca theme");
        assert_eq!(laporan.diff.created, 0, "tidak ada node yang lahir ulang");
        assert_eq!(laporan.diff.removed, 0);
        for q in kotak(&ui) {
            assert_eq!(q.background, gelap.color.surface);
        }
        assert!(ui.is_idle());
    }
}
