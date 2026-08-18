//! Demo page: **Tier 0 primitives** — the type scale and the container box
//! (`KOMPONEN.md` Tier 0), both written in the utility vocabulary of §2.6.
//!
//! The two oldest pages of this gallery (the typography specimen and the
//! squircle/arc comparison) assemble a `Scene` by hand, because they predate
//! the widget layer. This page shows the same two things **through the widget
//! layer**, which is what makes them usable inside the shell — and, more
//! importantly, is what makes them a test of the real path:
//! `text()` measuring itself through `silka-text`, and a container that
//! resolves its corners and shadows from tokens.
//!
//! It is also the page where the vocabulary earns its keep the most visibly.
//! What a heading used to take, and what it takes now:
//!
//! ```text
//! // before
//! text(judul)
//!     .size(t.typography.caption1.size)
//!     .weight(FontWeight::SEMIBOLD)
//!     .tracking(t.typography.caption1.tracking)
//!     .color(t.color.tertiary_label)
//!     .single_line()
//!
//! // after
//! text(judul)
//!     .font(FontToken::Caption1)
//!     .font_semibold()
//!     .text_color(ColorToken::TertiaryLabel)
//!     .single_line()
//! ```
//!
//! Note what disappeared with the theme lookups: the chance of taking the
//! *size* of one role and the *tracking* of another. `font()` moves the four
//! properties of a typographic role together, because separating them is how a
//! type scale drifts.
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
use silka_core::tree::CrossAlign;
use silka_core::view::{div, fixed, View};
use silka_paint::{CornerStyle, Corners};
#[cfg(test)]
use silka_theme::TypeStyle;
use silka_theme::{ColorToken, FontToken, RadiusToken, ShadowToken, Theme};
use silka_widgets::{active_fonts, text};

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
/// Returned as **tokens**: the page names eleven roles and holds not one
/// number, which is the whole claim — a preset switch changes all eleven at
/// once.
pub fn skala() -> [FontToken; 11] {
    FontToken::ALL
}

/// The same scale resolved against one theme — for the tests, which have to
/// compare numbers somewhere.
#[cfg(test)]
pub fn skala_terpakai(t: &Theme) -> [(&'static str, TypeStyle); 11] {
    skala().map(|token| (token.name(), t.typography.get(token)))
}

/// The radius/elevation pairs shown in the container section.
pub fn tingkat() -> [(RadiusToken, ShadowToken); 4] {
    [
        (RadiusToken::Sm, ShadowToken::Sm),
        (RadiusToken::Md, ShadowToken::Sm),
        (RadiusToken::Lg, ShadowToken::Md),
        (RadiusToken::Xl, ShadowToken::Lg),
    ]
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterised at the real screen resolution; the logical sizes
    // below do not change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    div()
        .items_center()
        .justify_center()
        .gap_5()
        .p_8()
        .child(
            text(JUDUL)
                .font(FontToken::Title2)
                .font_semibold()
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .child(
            text(
                "Dua primitif Tier 0 yang dipakai semua komponen lain: satu \
                 baris teks yang mengukur dirinya sendiri, dan satu kotak yang \
                 mengambil sudut serta bayangannya dari token. Ganti preset di \
                 bilah atas — seluruh halaman ini ikut berubah tanpa satu pun \
                 angka di berkas ini berubah.",
            )
            .text_base()
            .text_color(ColorToken::SecondaryLabel)
            .max_width(t.space(LEBAR_LANGKAH)),
        )
        .child(judul_bagian("Skala tipografi"))
        .child(spesimen())
        .child(judul_bagian("Sudut & bayangan"))
        .child(kartu_kartu(&t))
        .into()
}

/// A section heading.
fn judul_bagian(judul: &str) -> View {
    text(judul)
        .font(FontToken::Caption1)
        .font_semibold()
        .text_color(ColorToken::TertiaryLabel)
        .single_line()
        .into()
}

/// The type scale: token name on the left, the sample rendered on the right.
fn spesimen() -> View {
    div()
        .items_start()
        .gap_2()
        .children(skala().map(|token| {
            div()
                .flex()
                .cross(CrossAlign::Baseline)
                .gap_3()
                .child(
                    text(token.name())
                        .text_xs()
                        .text_color(ColorToken::TertiaryLabel)
                        .single_line(),
                )
                .child(
                    text(CONTOH)
                        // One call for size, line height, weight and tracking:
                        // the four properties of a typographic role travel
                        // together.
                        .font(token)
                        .text_color(ColorToken::Label)
                        .single_line(),
                )
        }))
        .into()
}

/// Four radius steps × two corner shapes.
fn kartu_kartu(t: &Theme) -> View {
    let sisi = t.space(KARTU);
    div()
        .items_start()
        .gap_3()
        .children(tingkat().map(|(radius, elevasi)| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    text(radius.name())
                        .text_xs()
                        .text_color(ColorToken::TertiaryLabel)
                        .single_line(),
                )
                // This page (like `cards`) is one of the few allowed to pick a
                // corner *shape* by hand, because comparing the two shapes is
                // precisely its job. Everything else — the radius included — is
                // a token.
                .child(kartu(t, sisi, radius, CornerStyle::squircle(), elevasi))
                .child(kartu(t, sisi, radius, CornerStyle::Arc, elevasi))
        }))
        .into()
}

/// One specimen card.
fn kartu(
    t: &Theme,
    sisi: f32,
    radius: RadiusToken,
    style: CornerStyle,
    elevasi: ShadowToken,
) -> View {
    fixed(sisi, sisi)
        .bg(ColorToken::Surface)
        .rounded_raw(Corners::uniform(t.radius.get(radius), style))
        .border_1()
        .border_color(ColorToken::Separator)
        .elevation(elevasi)
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

    fn ui(theme: Theme) -> AppRuntime {
        let mut ui =
            headless_app(theme, move |cx| halaman(cx)).sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        ui
    }

    #[test]
    fn skala_naik_monoton() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let langkah = skala_terpakai(&t);
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
        let a = skala_terpakai(&Theme::cupertino(Appearance::Light));
        let b = skala_terpakai(&Theme::tailwind(Appearance::Light));
        assert!(
            a.iter().zip(b.iter()).any(|(x, y)| x.1 != y.1),
            "kedua preset memberi skala tipografi yang identik — token tidak \
             berpengaruh apa-apa"
        );
    }

    #[test]
    fn halaman_menggambar_teks_dan_kotak() {
        let ui = ui(Theme::cupertino(Appearance::Dark));
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
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                assert_eq!(ui(t).scene().clear_color(), t.color.background);
            }
        }
    }

    /// The utilities really do resolve against the theme the frame installs —
    /// not against `Theme::default`, which is the failure mode a page written
    /// this way would otherwise hide.
    #[test]
    fn kotak_spesimen_memakai_warna_dan_hairline_dari_token() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let ui = ui(t);
                let kotak: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(kotak.len(), tingkat().len() * 2);
                for q in kotak {
                    assert_eq!(q.background, t.color.surface, "{preset:?} {appearance:?}");
                    assert_eq!(q.border_color, t.color.separator);
                    assert_eq!(q.border_width, t.space(0.25), "border_1 = hairline");
                }
            }
        }
    }

    /// Both corner shapes, at exactly the same nominal radius — the comparison
    /// this page exists for.
    #[test]
    fn tiap_baris_memasangkan_squircle_dengan_arc_beradius_sama() {
        let t = Theme::cupertino(Appearance::Dark);
        let ui = ui(t);
        let kotak: Vec<_> = ui
            .scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect();
        for (pasangan, (radius, _)) in kotak.chunks(2).zip(tingkat()) {
            assert_eq!(pasangan[0].corners.style, CornerStyle::squircle());
            assert_eq!(pasangan[1].corners.style, CornerStyle::Arc);
            assert_eq!(pasangan[0].corners.radii, pasangan[1].corners.radii);
            assert_eq!(pasangan[0].corners.radii.max(), t.radius.get(radius));
        }
    }
}
