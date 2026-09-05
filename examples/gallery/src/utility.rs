//! Demo page: **the utility vocabulary itself** (REKOMENDASI §2.6) — a living
//! reference for the spelling every other page is written in.
//!
//! The other pages show *components*; this one shows the **grammar** they are
//! assembled with, and it is deliberately written the way it documents:
//! `div()`, `p_*`, `gap_*`, `rounded_*`, `shadow_*`, `bg()` and the closure
//! states, with not a single number and not a single colour literal in the
//! file.
//!
//! Four sections, one per family:
//!
//! | Section | What to look at |
//! |---|---|
//! | **Spacing** | Every step of the 4pt scale as real padding. `p_4()` is 4 *steps* (16pt), not 4 points |
//! | **Radius** | The same `rounded_*()` call in both presets: a squircle under Cupertino, a circular arc under Tailwind (§2.7) — switch the preset in the top bar and watch the shape change while the code does not |
//! | **Shadow** | The five elevation tokens as the paired ambient + key shadows they really are (§3.6) |
//! | **States** | `hover`/`pressed`/`focused`/`disabled` — hover a tile, hold it down, and Tab into it. None of them jumps: each is a spring owned by the system (§2.6 discipline #2, §3.5) |
//!
//! The point of the states section is the thing a screenshot cannot show. Every
//! tile there asks only *what* it should look like in a state; not one of them
//! says how long the transition takes, and there is no way to say it — the
//! duration belongs to the design system, not to the call site.

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::view::{div, fixed, interactive, View};
use silka_theme::{ColorToken, FontToken, Preset, RadiusToken, ShadowToken, SpaceToken, Theme};
use silka_widgets::{active_fonts, text};

/// The page title.
pub const JUDUL: &str = "Utility vocabulary";

/// The a11y name of the hover tile — also what the tests aim at.
pub const TILE_HOVER: &str = "Hover box";
/// The a11y name of the pressed tile.
pub const TILE_TEKAN: &str = "Pressed box";
/// The a11y name of the focus tile.
pub const TILE_FOKUS: &str = "Focused box";
/// The a11y name of the disabled tile.
pub const TILE_MATI: &str = "Disabled box";

/// Which spacing steps get a row of their own.
///
/// Not all seventeen: the page is a reference, not a table dump, and the big
/// end of the scale is layout spacing rather than padding.
pub const LANGKAH: [SpaceToken; 9] = [
    SpaceToken::None,
    SpaceToken::Px,
    SpaceToken::S1,
    SpaceToken::S2,
    SpaceToken::S3,
    SpaceToken::S4,
    SpaceToken::S6,
    SpaceToken::S8,
    SpaceToken::S12,
];

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterised at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    div()
        .items_start()
        .gap_8()
        .p_8()
        .child(
            text(JUDUL)
                .font(FontToken::Title2)
                .font_semibold()
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .child(
            text(keterangan(t.preset))
                .text_base()
                .text_color(ColorToken::SecondaryLabel)
                .max_width(t.space(120.0)),
        )
        .child(bagian("Spacing · 4pt scale", spacing()))
        .child(bagian("Radius · per preset", radius()))
        .child(bagian("Shadow · elevation", shadow()))
        .child(bagian("State · hover, pressed, focused", keadaan()))
        .into()
}

/// The intro line, which names the preset currently in force — the one place
/// on the page where the preset is mentioned at all.
fn keterangan(preset: Preset) -> String {
    let bentuk = match preset {
        Preset::Cupertino => "squircle",
        Preset::Tailwind => "circular arc",
    };
    format!(
        "Every value on this page is a token, not a number. Active preset: \
         {preset:?} — its corners are drawn as {bentuk}. Switch the preset in \
         the top bar: the shapes and the sizes change, the page's code does \
         not."
    )
}

/// A section: heading plus content.
fn bagian(judul: &str, isi: View) -> View {
    div()
        .items_start()
        .gap_3()
        .child(
            text(judul)
                .font(FontToken::Caption1)
                .font_semibold()
                .text_color(ColorToken::TertiaryLabel)
                .single_line(),
        )
        .child(isi)
        .into()
}

/// A small caption under a specimen.
fn nama(teks: &str) -> View {
    text(teks)
        .text_xs()
        .text_color(ColorToken::TertiaryLabel)
        .single_line()
        .into()
}

/// The 4pt scale, shown as what it actually is: padding.
///
/// Each row is `p(token)` around the same inner block, so the token is not
/// described — it is *applied*.
fn spacing() -> View {
    div()
        .items_start()
        .gap_2()
        .children(LANGKAH.map(|token| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(nama(token.name()))
                .child(
                    div()
                        .p(token)
                        .bg(ColorToken::AccentMuted)
                        .rounded_sm()
                        .child(fixed(96.0, 12.0).bg(ColorToken::Accent).rounded_sm()),
                )
        }))
        .into()
}

/// The radius tokens — the section that changes shape with the preset.
fn radius() -> View {
    div()
        .flex()
        .items_start()
        .gap_4()
        .children(RadiusToken::ALL.map(|token| {
            div()
                .items_center()
                .gap_2()
                .child(
                    fixed(72.0, 72.0)
                        .bg(ColorToken::Surface)
                        // The whole point: one call, two geometries.
                        .rounded(token)
                        .border_1()
                        .border_color(ColorToken::Border),
                )
                .child(nama(token.name()))
        }))
        .into()
}

/// The elevation tokens, on a surface that is meant to float.
fn shadow() -> View {
    div()
        .flex()
        .items_start()
        // Wide gaps on purpose: an elevation is only readable when its shadow
        // has room to fall on the background rather than on its neighbour.
        .gap_6()
        .children(ShadowToken::ALL.map(|token| {
            div()
                .items_center()
                .gap_2()
                .child(
                    fixed(96.0, 64.0)
                        .bg(ColorToken::SurfaceElevated)
                        .rounded_lg()
                        .elevation(token),
                )
                .child(nama(token.name()))
        }))
        .into()
}

/// The interaction states — the section that has to be *used*, not looked at.
fn keadaan() -> View {
    div()
        .flex()
        .items_stretch()
        .gap_4()
        .child(
            tile(TILE_HOVER, "Point at it")
                .hover(|s| {
                    s.bg(ColorToken::SurfaceHover)
                        .border_color(ColorToken::Border)
                })
                .tab_order(1),
        )
        .child(
            tile(TILE_TEKAN, "Hold the mouse button")
                .hover(|s| s.bg(ColorToken::SurfaceHover))
                // `scale` is decorative motion: under reduced motion it does not
                // happen at all, while the colour change keeps running (§3.5).
                .pressed(|s| s.bg(ColorToken::AccentMuted).scale(0.96))
                .tab_order(2),
        )
        .child(
            tile(TILE_FOKUS, "Tab to here")
                .focused(|s| s.ring(ColorToken::FocusRing))
                .tab_order(3),
        )
        .child(
            tile(TILE_MATI, "Not usable")
                .disabled(true)
                .disabled_style(|s| s.bg(ColorToken::SurfaceSunken)),
        )
        .into()
}

/// One state tile: the resting look every tile shares.
fn tile(
    label: &str,
    keterangan: &str,
) -> silka_core::view::Builder<silka_core::view::InteractiveProps> {
    interactive(
        div()
            .items_start()
            .justify_center()
            .gap_1()
            .px_5()
            .py_4()
            .child(
                text(label)
                    .text_sm()
                    .font_semibold()
                    .text_color(ColorToken::Label)
                    .single_line(),
            )
            .child(
                text(keterangan)
                    .text_xs()
                    .text_color(ColorToken::SecondaryLabel)
                    .single_line(),
            ),
    )
    .label(label)
    .bg(ColorToken::Surface)
    .rounded_lg()
    .border_1(ColorToken::Separator)
    .elevation(ShadowToken::Sm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerEvent, PointerPhase};
    use silka_paint::CornerStyle;
    use silka_paint::{Command, Point, Quad, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::Appearance;
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(1100.0, 1400.0);

    fn ui(theme: Theme) -> AppRuntime {
        let mut ui = headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        ui
    }

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

    fn kotak_label(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    #[test]
    fn halaman_menggambar_teks_kotak_dan_bayangan() {
        let ui = ui(Theme::cupertino(Appearance::Dark));
        let perintah = ui.scene().commands();
        assert!(perintah.iter().any(|c| matches!(c, Command::GlyphRun(_))));
        assert!(perintah.iter().any(|c| matches!(c, Command::Quad(_))));
        assert!(perintah.iter().any(|c| matches!(c, Command::Shadow(_))));
    }

    /// The spacing section really applies the scale: each step is 4pt wider on
    /// each side than the one before it, in whichever preset.
    #[test]
    fn setiap_langkah_spacing_menambah_padding_sebenarnya() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let ui = ui(t);
            // The padded boxes are the ones carrying the `accent_muted` colour.
            let lebar: Vec<f32> = kotak(&ui)
                .iter()
                .filter(|q| q.background == t.color.accent_muted)
                .map(|q| q.rect.size.width)
                .collect();
            assert_eq!(lebar.len(), LANGKAH.len(), "{preset:?}");
            for (i, token) in LANGKAH.into_iter().enumerate() {
                let diharapkan = 96.0 + 2.0 * t.spacing.get(token);
                assert!(
                    (lebar[i] - diharapkan).abs() < 0.5,
                    "{preset:?} langkah {}: {} bukan {diharapkan}",
                    token.name(),
                    lebar[i]
                );
            }
        }
    }

    /// The claim the section exists for: the same `rounded_*()` call is a
    /// squircle in one preset and an arc in the other, at the preset's own
    /// numbers.
    #[test]
    fn radius_mengikuti_bentuk_dan_angka_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let ui = ui(t);
            let spesimen: Vec<Quad> = kotak(&ui)
                .into_iter()
                .filter(|q| q.rect.size == Size::new(72.0, 72.0))
                .collect();
            assert_eq!(spesimen.len(), RadiusToken::ALL.len(), "{preset:?}");
            for (q, token) in spesimen.iter().zip(RadiusToken::ALL) {
                // A zero radius has no shape to speak of, so `none` is the one
                // token that does not carry the preset's curve.
                if token != RadiusToken::None {
                    assert_eq!(q.corners.style, t.radius.style, "{preset:?}");
                }
                // `full` asks for a pill, which is clamped to half the box —
                // so what may be compared is what fits, not what was asked.
                let diminta = t.radius.get(token).min(q.rect.size.min_side() * 0.5);
                assert!(
                    (q.corners.radii.max() - diminta).abs() < 0.5,
                    "{preset:?} {}: {} bukan {diminta}",
                    token.name(),
                    q.corners.radii.max()
                );
            }
        }
        assert_ne!(
            Theme::cupertino(Appearance::Light).radius.style,
            CornerStyle::Arc,
            "preset Cupertino harus squircle — kalau tidak, bagian ini tidak \
             membuktikan apa pun"
        );
    }

    /// Elevation is monotonic: a token that stands for "higher" must blur more.
    #[test]
    fn elevasi_naik_dari_none_sampai_xl() {
        let t = Theme::cupertino(Appearance::Light);
        let ui = ui(t);
        let blur: Vec<f32> = ui
            .scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Shadow(s) if s.rect.size == Size::new(96.0, 64.0) => Some(s.blur),
                _ => None,
            })
            .collect();
        assert!(!blur.is_empty(), "bagian shadow tidak menggambar apa pun");
        // Two shadows per specimen (ambient + key); the ambient one comes first.
        let ambient: Vec<f32> = blur.iter().copied().step_by(2).collect();
        assert!(
            ambient.windows(2).all(|w| w[1] >= w[0]),
            "elevasi tidak monoton: {ambient:?}"
        );
        assert!(ambient.last() > ambient.first());
    }

    /// Every state tile is announced, and the disabled one is announced as
    /// disabled rather than silently inert (§3.8).
    #[test]
    fn tiap_tile_state_terbaca_screen_reader() {
        let ui = ui(Theme::cupertino(Appearance::Dark));
        let pohon = ui.access_tree();
        for label in [TILE_HOVER, TILE_TEKAN, TILE_FOKUS, TILE_MATI] {
            pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
        }
        assert!(
            pohon.find_label(TILE_MATI).unwrap().node.disabled,
            "tile disabled tidak diumumkan sebagai nonaktif"
        );
        assert!(!pohon.find_label(TILE_HOVER).unwrap().node.disabled);
    }

    /// The reference has to be honest about the thing it advertises: hovering a
    /// tile transitions, it does not cut.
    #[test]
    fn tile_hover_bertransisi_bukan_melompat() {
        let t = Theme::cupertino(Appearance::Dark);
        let mut ui = ui(t);
        let mut jam = Instant::now();
        frame(&mut ui, jam);

        let tile = kotak_label(&ui, TILE_HOVER);
        let warna_tile = |ui: &AppRuntime| {
            kotak(ui)
                .into_iter()
                .find(|q| (q.rect.min_x() - tile.min_x()).abs() < 0.5 && q.rect.size == tile.size)
                .map(|q| q.background)
                .expect("kotak tile hover tidak ketemu")
        };
        assert_eq!(warna_tile(&ui), t.color.surface);

        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(tile.center().x, tile.center().y),
            Duration::ZERO,
        )));
        for _ in 0..2 {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
        }
        let tengah = warna_tile(&ui);
        assert_ne!(tengah, t.color.surface, "tidak bergerak sama sekali");
        assert_ne!(
            tengah, t.color.surface_hover,
            "sampai di tujuan dalam dua frame — itu lompatan"
        );

        let mut n = 0;
        while !ui.is_idle() {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
            n += 1;
            assert!(n < 600, "transisi tidak pernah selesai");
        }
        assert_eq!(warna_tile(&ui), t.color.surface_hover);
    }
}
