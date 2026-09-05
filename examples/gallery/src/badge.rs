//! Demo page: **badge** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Correct in both presets | `--preset tailwind`: the pill keeps [`silka_theme::RadiusToken::Full`], so only the type and the tint change |
//! | Dark mode | `--appearance dark`: every tone is a token, so all five move together |
//! | The tone vocabulary | five tones × three variants, in one grid — "danger" has to look the same here as on a filter chip |
//! | A count cannot overflow its pill | `99+` instead of a four-digit number stretching a dot across a toolbar |
//! | One character is a circle | the `3` pill is as wide as it is tall, because the minimum width **is** the height |
//! | AccessKit node | a badge with a name is announced as "Status: Paid"; one without is structural, so its text is not read twice |
//! | Reduced motion | nothing here moves: a badge is a statement, not a control |
//!
//! ```text
//! cargo run -p silka-gallery -- --page badge
//! cargo run -p silka-gallery -- --page badge --preset tailwind --appearance light
//! ```
//!
//! The page deliberately shows the **whole** matrix rather than a pretty
//! selection: a tone that only looks right next to its own kind is a tone that
//! will surprise someone in a table row.

use silka_core::app::BuildCtx;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{badge, badge_count, BadgeTone, BadgeVariant};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Badge";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A status pill: five tones × three variants, plus a \
    counter that cannot overflow. A badge states something — the clickable \
    thing is a ghost-variant button, and mixing the two turns a status into a \
    mysterious button.";

/// The a11y name of the status pill the tests look for.
pub const NAMA_STATUS: &str = "Status: Paid";
/// The a11y name of the unread counter.
pub const NAMA_HITUNG: &str = "Unread messages";
/// The counter that is deliberately larger than its cap.
pub const HITUNG_BESAR: u64 = 1_240;
/// The cap the large counter is written against.
pub const BATAS: u64 = 99;

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [matriks(&t), penghitung(&t), dalam_kalimat(&t)],
    )
}

/// Every tone against every variant — the grid that makes an inconsistent tone
/// impossible to miss.
fn matriks(t: &Theme) -> View {
    let baris = BadgeVariant::ALL.map(|variant| {
        let pil = BadgeTone::ALL.map(|tone| {
            View::from(
                badge(tone.name())
                    .key(format!("{}-{}", variant.name(), tone.name()))
                    .tone(tone)
                    .variant(variant),
            )
        });
        View::from(
            row([
                kepala::catatan(t, variant.name()),
                View::from(row(pil).spacing(t.space(2.0)).cross(CrossAlign::Center)),
            ])
            .spacing(t.space(4.0))
            .cross(CrossAlign::Center),
        )
    });

    kepala::spesimen(
        t,
        "Tone × variant",
        [View::from(column(baris).spacing(t.space(3.0)))],
    )
}

/// Counts, including one that has to be capped.
fn penghitung(t: &Theme) -> View {
    kepala::spesimen(
        t,
        "Counter",
        [View::from(
            row([
                View::from(badge_count(3).tone(BadgeTone::Accent)),
                View::from(badge_count(12).tone(BadgeTone::Accent)),
                View::from(
                    badge_count(HITUNG_BESAR)
                        .max_count(HITUNG_BESAR, BATAS)
                        .tone(BadgeTone::Danger)
                        .label(NAMA_HITUNG),
                ),
                View::from(badge("Draft").dot(true)),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center),
        )],
    )
}

/// A badge where badges actually live: at the end of a line of text.
fn dalam_kalimat(t: &Theme) -> View {
    kepala::spesimen(
        t,
        "Inline",
        [View::from(
            row([
                kepala::catatan(t, "Invoice #001280"),
                View::from(
                    badge("Paid")
                        .tone(BadgeTone::Success)
                        .soft()
                        .label(NAMA_STATUS),
                ),
            ])
            .spacing(t.space(2.0))
            .main(MainAlign::Start)
            .cross(CrossAlign::Center),
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_paint::{Command, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(900.0, 760.0);

    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
    }

    #[test]
    fn pil_bernama_dibacakan_sekali_dan_pil_polos_tidak_dua_kali() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        for nama in [NAMA_STATUS, NAMA_HITUNG] {
            assert!(
                pohon.find_label(nama).is_some(),
                "{nama} hilang dari pohon a11y:\n{}",
                pohon.dump()
            );
        }
        // A pill with no name of its own is still readable — its text is the
        // whole of what it says.
        assert!(
            pohon.find_label(BadgeTone::Neutral.name()).is_some(),
            "pil tanpa nama sama sekali tidak terbaca:\n{}",
            pohon.dump()
        );
        // …but a *named* pill must not be announced twice: the name replaces
        // the text rather than joining it.
        assert!(
            pohon.find_label("Paid").is_none(),
            "pil bernama dibacakan dua kali:\n{}",
            pohon.dump()
        );
        assert!(ui.is_idle(), "badge tidak menganimasikan apa pun");
    }

    #[test]
    fn penghitung_besar_dipotong_bukan_dibiarkan_meluber() {
        // The page's only claim about text: what the capped counter says.
        assert_eq!(silka_widgets::format_count(HITUNG_BESAR, BATAS), "99+");
        assert_eq!(silka_widgets::format_count(12, BATAS), "12");
    }

    #[test]
    fn setiap_warna_pil_datang_dari_token_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                let sah = [
                    t.color.label,
                    t.color.secondary_label,
                    t.color.tertiary_label,
                    t.color.on_accent,
                    t.color.accent,
                    t.color.success,
                    t.color.warning,
                    t.color.destructive,
                ];
                for c in ui.scene().commands() {
                    if let Command::GlyphRun(r) = c {
                        assert!(
                            sah.contains(&r.color),
                            "warna teks lepas dari token: {:?} ({preset:?} {appearance:?})",
                            r.color
                        );
                    }
                }
            }
        }
    }
}
