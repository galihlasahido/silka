//! Demo page: **the card grid, written in the utility vocabulary** (§2.6).
//!
//! Its visual content is still the one from [`crate::cards`] — squircles on the
//! left, arcs on the right, radius and elevation growing per row — but nothing
//! about the route there is hand-made any more:
//!
//! 1. **No `Scene` is assembled here.** The page is a view tree; the scene is
//!    born from `signals → view-diff → layout → paint` inside
//!    [`silka_core::app::AppRuntime`] (§2, §3.5).
//! 2. **No layout arithmetic.** `crate::cards` computes
//!    `(width - padding * 2 - gap) * 0.5` for every cell; here the whole page
//!    is `div().p_6().gap_4()` plus `expanded()`, and Taffy does the division
//!    (§3.4).
//! 3. **No `theme.` in the styling.** `bg(ColorToken::Surface)`,
//!    `border_1(ColorToken::Separator)`, `elevation(ShadowToken::Md)` name the
//!    *role*; the ambient theme turns them into numbers while the view is being
//!    built (§2.6, §2.7).
//!
//! The same card, before and after the vocabulary (this file's own history):
//!
//! ```text
//! // before — four values, four theme lookups, three imports
//! fixed(0.0, 0.0)
//!     .background(t.color.surface)
//!     .corners(Corners::uniform(radius, style))
//!     .border(t.space(0.25), t.color.separator)
//!     .shadow(shadow)
//!
//! // after — four roles, no theme lookup, and three interaction states on top
//! interactive(fixed(0.0, 0.0))
//!     .bg(ColorToken::Surface)
//!     .border_1(ColorToken::Separator)
//!     .elevation(ShadowToken::Md)
//!     .hover(|s| s.bg(ColorToken::SurfaceHover))
//!     .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.98))
//!     .focused(|s| s.ring(ColorToken::FocusRing))
//! ```
//!
//! …and the page frame lost its arithmetic entirely: `.spacing(t.space(4.0))`
//! `.cross(CrossAlign::Stretch)` `.padding(Insets::all(t.space(6.0)))` became
//! `.gap_4()` `.p_6()`, with `Insets` and `CrossAlign` no longer imported.
//!
//! ## The cards used to jump
//!
//! Every card is an `interactive(…)` with `hover`/`pressed`/`focused`, and each
//! of those states is a spring rather than a cut — the point of the
//! `utility-spring` milestone. Before it, `Interactive` picked a color with an
//! `if` chain, so an application-written card (exactly this one) snapped between
//! two colors within a single frame while first-party widgets, each carrying
//! their own spring, glided. Motion now belongs to the system: this page asks
//! for **no** duration, no curve, no timer, and still transitions.
//!
//! Under reduced motion the colors land instantly and the press scale does not
//! happen at all — decorative motion is dropped, informative motion is not
//! (§3.5). Nothing on this page has to know that either.

use silka_core::app::{component, BuildCtx};
use silka_core::signals::{Key, Signal};
use silka_core::view::{div, expanded, fixed, interactive, View};
use silka_paint::{CornerStyle, Corners};
use silka_theme::{ColorToken, RadiusToken, ShadowToken, Theme};

/// How many card rows (one row = one radius token + one elevation token).
pub const BARIS: usize = 4;

/// The view tree for the whole page — this is what gets handed to `run_app`.
///
/// Read in the root scope: a theme change rebuilds this page in its entirety,
/// which is exactly what we want since every value here is a token.
pub fn halaman(cx: &BuildCtx) -> View {
    // The read is the subscription. The *values* are not taken from `t`: the
    // utilities below resolve their own tokens against the same theme, which is
    // ambient for the duration of this build pass.
    let _t: Theme = cx.expect_env::<Signal<Theme>>().get();

    div()
        .p_6()
        .gap_4()
        .children((0..BARIS).map(|baris| {
            expanded(
                div()
                    .flex()
                    .items_stretch()
                    .gap_4()
                    .child(expanded(kartu(baris, 0)))
                    .child(expanded(kartu(baris, 1))),
            )
        }))
        .into()
}

/// A single card as its own component.
///
/// Each card owns its scope, so hovering one rebuilds nothing at all — the
/// state lives in the render node and moves on a spring, without a rebuild.
fn kartu(baris: usize, kolom: usize) -> View {
    component(Key::num((baris * 2 + kolom) as i64), move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let (radius, elevasi) = gaya_baris(baris);
        // Squircle on the left, arc on the right. This is the one escape hatch
        // on the page and it is deliberate: comparing the two shapes is the
        // page's whole job, so the shape cannot come from the preset here. The
        // *radius* still does.
        let bentuk = if kolom == 0 {
            CornerStyle::squircle()
        } else {
            CornerStyle::Arc
        };

        // Zero size: `expanded()` hands down tight constraints, so the card
        // fills its cell. The layout numbers belong to the layout engine.
        interactive(fixed(0.0, 0.0))
            .label(nama(baris, kolom))
            .corners(Corners::uniform(t.radius.get(radius), bentuk))
            .bg(ColorToken::Surface)
            .border_1(ColorToken::Separator)
            .elevation(elevasi)
            .hover(|s| s.bg(ColorToken::SurfaceHover))
            .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.98))
            .focused(|s| s.ring(ColorToken::FocusRing))
            .into()
    })
}

/// Radius + elevation for a row — as **tokens**, so the page never holds a
/// number.
pub fn gaya_baris(baris: usize) -> (RadiusToken, ShadowToken) {
    match baris {
        0 => (RadiusToken::Sm, ShadowToken::Sm),
        1 => (RadiusToken::Md, ShadowToken::Sm),
        2 => (RadiusToken::Lg, ShadowToken::Md),
        _ => (RadiusToken::Xl, ShadowToken::Lg),
    }
}

/// The a11y name of one card — also what the tests click, so what a test aims
/// at is exactly what a screen reader announces (§3.8).
pub fn nama(baris: usize, kolom: usize) -> String {
    let (radius, _) = gaya_baris(baris);
    let bentuk = if kolom == 0 { "squircle" } else { "arc" };
    format!("Kartu {} {bentuk}", radius.name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Command, Point, Quad, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(1024.0, 720.0);

    /// A headless app assembled exactly the way `run_app` does it.
    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// One complete frame, animation tick included — the same order as the
    /// shell (`silka_platform::run_app_with`).
    fn frame(ui: &mut AppRuntime, waktu: Instant) {
        ui.animate_at(waktu, silka_widgets::advance);
        ui.frame();
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

    /// A card's rectangle **according to the accessibility tree** — the tests
    /// aim where a screen reader announces (§3.8).
    fn kotak_kartu(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada kartu berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
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
    fn padding_dan_gap_datang_dari_skala_4pt() {
        // `p_6()` = 6 steps, `gap_4()` = 4 steps — in both presets, without the
        // page ever holding a number.
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let mut ui = ui(t);
            ui.frame();
            let k = kotak(&ui);
            assert_eq!(k[0].rect.min_x(), t.space(6.0), "{preset:?}");
            assert_eq!(k[0].rect.min_y(), t.space(6.0), "{preset:?}");
            assert_eq!(
                k[1].rect.min_x() - k[0].rect.max_x(),
                t.space(4.0),
                "{preset:?}"
            );
            assert!(
                (k[2].rect.min_y() - k[0].rect.max_y() - t.space(4.0)).abs() < 1e-3,
                "{preset:?}"
            );
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
    fn radius_dan_elevasi_naik_per_baris() {
        let t = Theme::cupertino(Appearance::Dark);
        let mut ui = ui(t);
        ui.frame();
        let k = kotak(&ui);
        let radius: Vec<f32> = k.chunks(2).map(|b| b[0].corners.radii.max()).collect();
        assert_eq!(
            radius,
            vec![t.radius.sm, t.radius.md, t.radius.lg, t.radius.xl]
        );
        assert!(radius.windows(2).all(|w| w[0] < w[1]), "{radius:?}");
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
                    // `border_1()` is the hairline token, one point in every
                    // preset.
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

        // A theme swap **re-aims** the same springs the hover uses, so the
        // cards cross-fade into dark mode instead of blinking. Pump the frames
        // the way the shell does until everything has settled.
        let mut jam = Instant::now();
        let mut n = 0;
        while !ui.is_idle() {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
            n += 1;
            assert!(n < 600, "transisi theme tidak pernah selesai");
        }
        // The new colour arrives resolved: the page said `ColorToken::Surface`
        // once and never mentioned a theme again.
        for q in kotak(&ui) {
            assert_eq!(q.background, gelap.color.surface);
        }
    }

    /// The regression this page exists to guard: a card must **not** land on
    /// its hover colour within the frame the pointer arrives.
    #[test]
    fn kartu_tidak_melompat_saat_hover_melainkan_bertransisi() {
        let t = Theme::cupertino(Appearance::Dark);
        let mut ui = ui(t);
        let mut jam = Instant::now();
        frame(&mut ui, jam);

        let sasaran = kotak_kartu(&ui, &nama(0, 0)).center();
        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(sasaran.x, sasaran.y),
            Duration::ZERO,
        )));

        // Two frames: the first one only re-aims the spring (the pointer
        // arrived between ticks), the second is the first one that moves it.
        for _ in 0..2 {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
        }
        let sekarang = kotak(&ui)[0].background;
        assert_ne!(
            sekarang, t.color.surface_hover,
            "warna hover tercapai dalam dua frame — ini lompatan, bukan transisi"
        );
        assert_ne!(sekarang, t.color.surface, "belum bergerak sama sekali");

        // …and it does finish, without anyone touching a timer.
        let mut n = 0;
        while !ui.is_idle() {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
            n += 1;
            assert!(n < 600, "transisi hover tidak pernah selesai");
        }
        assert!(n > 1, "transisi harus memakan lebih dari satu frame");
        assert_eq!(kotak(&ui)[0].background, t.color.surface_hover);
    }

    /// Pointer leaves halfway: the spring reverses from where it is, it does not
    /// restart from the resting colour.
    #[test]
    fn pointer_pergi_di_tengah_jalan_berbalik_tanpa_sambungan() {
        let t = Theme::cupertino(Appearance::Dark);
        let mut ui = ui(t);
        let mut jam = Instant::now();
        frame(&mut ui, jam);

        let sasaran = kotak_kartu(&ui, &nama(0, 0)).center();
        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(sasaran.x, sasaran.y),
            Duration::ZERO,
        )));
        for _ in 0..3 {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
        }
        let tengah = kotak(&ui)[0].background;

        // Out of the window entirely.
        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(-10.0, -10.0),
            Duration::from_millis(64),
        )));
        jam += Duration::from_millis(16);
        frame(&mut ui, jam);
        let sesudah = kotak(&ui)[0].background;
        assert_ne!(sesudah, tengah, "tidak bergerak balik");
        assert_ne!(sesudah, t.color.surface, "kembali seketika = lompatan");
    }

    /// Reduced motion: the colour lands at once (a half-faded card is worse than
    /// no transition), and the press scale never happens at all (§3.5).
    #[test]
    fn reduced_motion_mendarat_seketika_dan_tidak_mengecil() {
        let t = Theme::cupertino(Appearance::Light);
        let mut ui = ui(t);
        ui.set_motion(Motion::Reduced);
        let mut jam = Instant::now();
        frame(&mut ui, jam);
        let sebelum = kotak(&ui)[0].rect;

        let sasaran = kotak_kartu(&ui, &nama(0, 0)).center();
        let titik = Point::new(sasaran.x, sasaran.y);
        for e in [
            PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
        jam += Duration::from_millis(16);
        frame(&mut ui, jam);

        assert_eq!(
            kotak(&ui)[0].background,
            t.color.surface_pressed,
            "gerak dikurangi berarti mendarat, bukan memudar"
        );
        assert_eq!(
            kotak(&ui)[0].rect.size,
            sebelum.size,
            "scale adalah gerak dekoratif: di mode ini tidak terjadi sama sekali"
        );
    }
}
