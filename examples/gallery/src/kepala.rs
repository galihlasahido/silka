//! The furniture every demo page repeats: a title, the paragraph under it, and
//! the small heading above each specimen.
//!
//! It exists for one reason. Sixteen Tier 4/5 pages were written in one go, and
//! without a shared header each of them would have invented its own type size
//! for "title" — which is precisely the drift a gallery is supposed to *catch*.
//! One function means one answer, and a token regression in the heading shows
//! up on every page at once instead of on the one page someone remembered to
//! update.
//!
//! Nothing here knows anything about a component: it is text, spacing and
//! tokens, and not one number in it is a literal (§2.6, §2.7).

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{active_fonts, text};

/// How wide a paragraph is allowed to get, in spacing steps.
///
/// Roughly 70 characters at the body size: past that the eye loses the start of
/// the next line, which is a typographic fact rather than a design opinion.
pub const LEBAR_PARAGRAF: f32 = 120.0;

/// The theme for this build pass, with the glyph atlas already told which
/// screen it is rasterising for.
///
/// Every page opens with these two lines; putting them here means a page that
/// forgets the scale factor cannot exist (§3.3).
pub fn mulai(cx: &BuildCtx) -> Theme {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());
    t
}

/// The page title.
pub fn judul(t: &Theme, teks: &str) -> View {
    text(teks)
        .size(t.typography.title2.size)
        .weight(FontWeight::SEMIBOLD)
        // Negative tracking at large sizes — an SF habit (§3.6).
        .tracking(t.typography.title2.tracking)
        .color(t.color.label)
        .single_line()
        .into()
}

/// The paragraph under the title: what to look at, and what to try.
pub fn keterangan(t: &Theme, teks: &str) -> View {
    text(teks)
        .size(t.typography.body.size)
        .line_height(t.typography.body.line_height)
        .color(t.color.secondary_label)
        .max_width(t.space(LEBAR_PARAGRAF))
        .into()
}

/// The heading above one specimen.
pub fn bagian(t: &Theme, teks: &str) -> View {
    text(teks)
        .size(t.typography.subheadline.size)
        .weight(FontWeight::SEMIBOLD)
        .color(t.color.tertiary_label)
        .single_line()
        .into()
}

/// A line of prose inside a specimen — the caption that says what just
/// happened.
pub fn catatan(t: &Theme, teks: impl Into<String>) -> View {
    text(teks)
        .size(t.typography.callout.size)
        .color(t.color.tertiary_label)
        .max_width(t.space(LEBAR_PARAGRAF))
        .into()
}

/// Title, paragraph, then the specimens — the shape of every Tier 4/5 page.
pub fn halaman(
    t: &Theme,
    nama: &str,
    ringkasan: &str,
    isi: impl IntoIterator<Item = View>,
) -> View {
    let mut anak = vec![judul(t, nama), keterangan(t, ringkasan)];
    anak.extend(isi);
    column(anak)
        .spacing(t.space(6.0))
        // The alignment is the layout engine's job, not arithmetic on the page
        // (§3.4).
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .padding(Insets::all(t.space(8.0)))
        .into()
}

/// One specimen: its heading, then whatever it is showing off.
pub fn spesimen(t: &Theme, nama: &str, isi: impl IntoIterator<Item = View>) -> View {
    let mut anak = vec![bagian(t, nama)];
    anak.extend(isi);
    column(anak)
        .spacing(t.space(3.0))
        .cross(CrossAlign::Start)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::{Appearance, Preset};

    /// The header is the one place a Tier 4/5 page is allowed to name a type
    /// size, so it is the one place worth pinning: both presets have to answer,
    /// and the answers have to differ from one another.
    #[test]
    fn judul_selalu_lebih_besar_dari_paragrafnya_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            assert!(
                t.typography.title2.size > t.typography.body.size,
                "{preset:?}: judul halaman tidak lebih besar dari isinya"
            );
            assert!(
                t.typography.subheadline.size <= t.typography.body.size,
                "{preset:?}: judul bagian menandingi judul halaman"
            );
        }
    }

    #[test]
    fn lebar_paragraf_adalah_kelipatan_langkah_bukan_piksel_ajaib() {
        let t = Theme::cupertino(Appearance::Dark);
        assert_eq!(t.space(LEBAR_PARAGRAF), t.space(1.0) * LEBAR_PARAGRAF);
    }
}
