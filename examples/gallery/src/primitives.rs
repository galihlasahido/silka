//! Demo page: **Tier 0 primitives** — the type scale and the container box
//! (`KOMPONEN.md` Tier 0).
//!
//! The two oldest pages of this gallery (the typography specimen and the
//! squircle/arc comparison) assemble a `Scene` by hand, because they predate
//! the widget layer. This page shows the same two things **through the widget
//! layer**, which is what makes them usable inside the shell — and, more
//! importantly, is what makes them a test of the real path:
//! `text()` measuring itself through `silka-text`, and a container that
//! resolves its corners and shadows from tokens.
//!
//! | What it proves | How to check it |
//! |---|---|
//! | The type scale is a set of tokens, not eleven literals | Switch preset in the top bar: every line changes size, weight and tracking at once |
//! | Optical tracking at large sizes | `large_title` is visibly tighter than `caption2` — an SF habit (§3.6) |
//! | Squircle vs arc | The left card of each pair is a superellipse, the right one a circular arc, at exactly the same nominal radius |
//! | Layered shadows follow the shape | The shadow under a squircle is not the shadow of a rounded rectangle |
//! | Dark mode | Every colour on this page is a token, so nothing here is tuned for one appearance |

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, fixed, row, View};
use silka_paint::{CornerStyle, Corners, Insets, ShadowPair};
use silka_text::FontWeight;
use silka_theme::{Theme, TypeStyle};
use silka_widgets::{text, Fonts};

/// The page title.
pub const JUDUL: &str = "Teks & kontainer";

/// The sample line rendered at every step of the type scale.
pub const CONTOH: &str = "Sphinx of black quartz, judge my vow";

/// Width of the page content, in spacing steps.
const LEBAR_LANGKAH: f32 = 120.0;

/// Size of one specimen card, in spacing steps.
const KARTU: f32 = 16.0;

/// The steps of the type scale, from the smallest to the largest.
///
/// Returned as a function of the theme rather than a constant, because that is
/// the whole claim being made: the scale **is** the token set, so a preset
/// switch changes all eleven entries at once.
pub fn skala(t: &Theme) -> [(&'static str, TypeStyle); 11] {
    let ty = &t.typography;
    [
        ("caption2", ty.caption2),
        ("caption1", ty.caption1),
        ("footnote", ty.footnote),
        ("subheadline", ty.subheadline),
        ("callout", ty.callout),
        ("body", ty.body),
        ("headline", ty.headline),
        ("title3", ty.title3),
        ("title2", ty.title2),
        ("title1", ty.title1),
        ("large_title", ty.large_title),
    ]
}

/// The radius/elevation pairs shown in the container section.
pub fn tingkat(t: &Theme) -> [(&'static str, f32, ShadowPair); 4] {
    [
        ("sm", t.radius.sm, t.shadow.sm),
        ("md", t.radius.md, t.shadow.sm),
        ("lg", t.radius.lg, t.shadow.md),
        ("xl", t.radius.xl, t.shadow.lg),
    ]
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterised at the real screen resolution; the logical sizes
    // below do not change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Dua primitif Tier 0 yang dipakai semua komponen lain: satu \
                 baris teks yang mengukur dirinya sendiri, dan satu kotak yang \
                 mengambil sudut serta bayangannya dari token. Ganti preset di \
                 bilah atas — seluruh halaman ini ikut berubah tanpa satu pun \
                 angka di berkas ini berubah.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        judul_bagian(fonts, &t, "Skala tipografi"),
        spesimen(fonts, &t),
        judul_bagian(fonts, &t, "Sudut & bayangan"),
        kartu_kartu(fonts, &t),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// A section heading.
fn judul_bagian(fonts: &Fonts, t: &Theme, judul: &str) -> View {
    text(fonts, judul)
        .size(t.typography.caption1.size)
        .weight(FontWeight::SEMIBOLD)
        .tracking(t.typography.caption1.tracking)
        .color(t.color.tertiary_label)
        .single_line()
        .into()
}

/// The type scale: token name on the left, the sample rendered on the right.
fn spesimen(fonts: &Fonts, t: &Theme) -> View {
    column(skala(t).map(|(nama, gaya)| {
        View::from(
            row([
                View::from(
                    text(fonts, nama)
                        .size(t.typography.caption2.size)
                        .color(t.color.tertiary_label)
                        .single_line(),
                ),
                View::from(
                    text(fonts, CONTOH)
                        .size(gaya.size)
                        .weight(FontWeight(gaya.weight))
                        .tracking(gaya.tracking)
                        // `TypeStyle::line_height` is already a multiple of the
                        // font size, which is exactly what `text()` wants.
                        .line_height(gaya.line_height)
                        .color(t.color.label)
                        .single_line(),
                ),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Baseline),
        )
    }))
    .spacing(t.space(2.0))
    .cross(CrossAlign::Start)
    .into()
}

/// Four radius steps × two corner shapes.
fn kartu_kartu(fonts: &Fonts, t: &Theme) -> View {
    let sisi = t.space(KARTU);
    column(tingkat(t).map(|(nama, radius, shadow)| {
        View::from(
            row([
                View::from(
                    text(fonts, nama)
                        .size(t.typography.caption2.size)
                        .color(t.color.tertiary_label)
                        .single_line(),
                ),
                // This page (like `cards`/`reactive`) is one of the few allowed
                // to pick a corner shape by hand, because comparing the two
                // shapes is precisely its job. Everything else is a token.
                kartu(t, sisi, radius, CornerStyle::squircle(), shadow),
                kartu(t, sisi, radius, CornerStyle::Arc, shadow),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center),
        )
    }))
    .spacing(t.space(3.0))
    .cross(CrossAlign::Start)
    .into()
}

/// One specimen card.
fn kartu(t: &Theme, sisi: f32, radius: f32, style: CornerStyle, shadow: ShadowPair) -> View {
    fixed(sisi, sisi)
        .background(t.color.surface)
        .corners(Corners::uniform(radius, style))
        .border(t.space(0.25), t.color.separator)
        .shadow(shadow)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_paint::{Command, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(960.0, 900.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        let mut ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        ui
    }

    #[test]
    fn skala_naik_monoton() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let langkah = skala(&t);
            for pasangan in langkah.windows(2) {
                assert!(
                    pasangan[1].1.size >= pasangan[0].1.size,
                    "{preset:?}: '{}' ({}) lebih kecil daripada '{}' ({})",
                    pasangan[1].0,
                    pasangan[1].1.size,
                    pasangan[0].0,
                    pasangan[0].1.size
                );
            }
        }
    }

    #[test]
    fn judul_besar_lebih_rapat_daripada_teks_kecil() {
        // Optical tracking: at large sizes the letters move closer together,
        // never further apart (§3.6).
        let t = Theme::cupertino(Appearance::Light);
        assert!(t.typography.large_title.tracking <= t.typography.caption2.tracking);
    }

    #[test]
    fn skala_ikut_preset() {
        let a = skala(&Theme::cupertino(Appearance::Light));
        let b = skala(&Theme::tailwind(Appearance::Light));
        assert!(
            a.iter().zip(b.iter()).any(|(x, y)| x.1 != y.1),
            "kedua preset memberi skala tipografi yang identik — token tidak \
             berpengaruh apa-apa"
        );
    }

    #[test]
    fn halaman_menggambar_teks_dan_kotak() {
        let f = fonts();
        let ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let perintah = ui.scene().commands();
        assert!(
            perintah.iter().any(|c| matches!(c, Command::GlyphRun(_))),
            "tidak ada satu pun glyph tergambar"
        );
        assert!(
            perintah.iter().any(|c| matches!(c, Command::Quad(_))),
            "tidak ada satu pun kotak tergambar"
        );
        assert!(
            perintah.iter().any(|c| matches!(c, Command::Shadow(_))),
            "bayangan berlapis tidak tergambar"
        );
    }

    #[test]
    fn latar_selalu_token_background() {
        let f = fonts();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                assert_eq!(ui(t, &f).scene().clear_color(), t.color.background);
            }
        }
    }
}
