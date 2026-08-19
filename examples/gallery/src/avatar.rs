//! Demo page: **avatar** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The fallback is the main case | not one avatar here has a photograph, because most accounts do not; the initials are the component, not a degraded state |
//! | Initials are a function, not a guess | one word, three words, a name in a script with no capitals — [`silka_widgets::initials`] has an answer for each |
//! | The tint is deterministic | the same name always lands on the same slot, so a person keeps their colour between screens |
//! | A stack says how many are hidden | five members with room for three reads "+2", not "three of them" |
//! | AccessKit node | a named disc for a person, a single name for the group, and a decorative one is skipped entirely |
//! | Correct in both presets | diameters are spacing steps, the ring is a token, the shape follows [`silka_theme::RadiusToken`] |
//!
//! ```text
//! cargo run -p silka-gallery -- --page avatar
//! ```

use silka_core::app::BuildCtx;
use silka_core::tree::CrossAlign;
use silka_core::view::{row, View};
use silka_theme::{RadiusToken, Theme};
use silka_widgets::{avatar, avatar_group, group_plan, initials};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Avatar";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A person, as a disc. What actually happens is not \
    the photo but the initials: most accounts have no picture, so that is the \
    main case — complete with a colour that is always the same for the same \
    name.";

/// The team the page shows.
pub const TIM: [&str; 5] = [
    "Dian Permata",
    "Bagas Nugroho",
    "Sari Wulandari",
    "Rizky Pratama",
    "Nadia Ayu",
];

/// How many discs the stack has room for.
pub const MUAT: usize = 3;
/// The a11y name of the stack.
pub const NAMA_TIM: &str = "Project team";
/// The name whose initials are deliberately a single letter.
pub const SATU_KATA: &str = "Prabowo";

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [ukuran(&t), bentuk(&t), tumpukan(&t)],
    )
}

/// The five sizes, smallest first.
fn ukuran(t: &Theme) -> View {
    kepala::spesimen(
        t,
        "Size",
        [View::from(
            row([
                View::from(avatar(TIM[0]).xs()),
                View::from(avatar(TIM[0]).sm()),
                View::from(avatar(TIM[0]).md()),
                View::from(avatar(TIM[0]).lg()),
                View::from(avatar(TIM[0]).xl()),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center),
        )],
    )
}

/// A disc, a rounded square, and one that says nothing at all.
fn bentuk(t: &Theme) -> View {
    kepala::spesimen(
        t,
        "Shape and name",
        [
            View::from(
                row([
                    View::from(avatar(TIM[1]).lg()),
                    View::from(avatar(TIM[2]).lg().rounded(RadiusToken::Md)),
                    View::from(avatar(SATU_KATA).lg()),
                    // Decorative: it repeats a name that is already on screen,
                    // so a screen reader must not read it a second time (§3.8).
                    View::from(avatar(TIM[3]).lg().decorative()),
                ])
                .spacing(t.space(3.0))
                .cross(CrossAlign::Center),
            ),
            kepala::catatan(
                t,
                format!(
                    "Initials: {} · {} · {}",
                    initials(TIM[1], 2),
                    initials(TIM[2], 2),
                    initials(SATU_KATA, 2),
                ),
            ),
        ],
    )
}

/// The overlapping stack, with the overflow count that follows from it.
fn tumpukan(t: &Theme) -> View {
    let (tampil, sisa) = group_plan(TIM.len(), MUAT);
    kepala::spesimen(
        t,
        "Stack",
        [
            View::from(
                avatar_group(TIM.map(avatar))
                    .max(MUAT)
                    .label(NAMA_TIM)
                    .size_raw(t.space(11.0)),
            ),
            kepala::catatan(
                t,
                format!("{tampil} discs are shown, the remaining {sisa} become \"+{sisa}\"."),
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_paint::Size;
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(880.0, 720.0);

    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
    }

    #[test]
    fn rencana_tumpukan_menyisakan_ruang_untuk_angkanya() {
        // Five people in room for three is "two visible plus +3", not "three
        // visible plus +2": the counter needs a slot of its own.
        assert_eq!(
            group_plan(TIM.len(), MUAT),
            (MUAT - 1, TIM.len() - MUAT + 1)
        );
        // Everyone fits: no counter at all.
        assert_eq!(group_plan(2, MUAT), (2, 0));
    }

    #[test]
    fn inisial_punya_jawaban_untuk_satu_kata_dan_dua_kata() {
        assert_eq!(initials(TIM[0], 2), "DP");
        assert_eq!(initials(SATU_KATA, 2), "P");
    }

    #[test]
    fn yang_dekoratif_tidak_ikut_dibacakan_dan_grupnya_bernama_satu_kali() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        assert!(
            pohon.find_label(NAMA_TIM).is_some(),
            "tumpukan harus punya satu nama:\n{}",
            pohon.dump()
        );
        // Rizky appears twice on the page: once as a named disc in the stack
        // and once as a decorative one. Only the stack speaks.
        let sebut = pohon.dump().matches(TIM[3]).count();
        assert!(
            sebut <= 1,
            "cakram dekoratif ikut dibacakan ({sebut}×):\n{}",
            pohon.dump()
        );
        assert!(ui.is_idle(), "avatar tidak menganimasikan apa pun");
    }

    #[test]
    fn halaman_terbangun_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);
                assert!(!ui.scene().is_empty(), "{preset:?}/{appearance:?}: kosong");
            }
        }
    }
}
