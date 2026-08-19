//! Demo page: **skeleton** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The placeholder is the shape of what is coming | press "Load": the real card takes exactly the room the skeleton was holding, so nothing jumps |
//! | The shimmer is quads, not a gradient | `silka-paint` has no gradient command; the highlight is a handful of alpha-stepped quads and looks the same |
//! | Correct in both presets | base and highlight are [`silka_theme::ColorToken::SurfaceSunken`] and `SurfaceHover`; nothing else has a colour |
//! | AccessKit node | hidden by default — a screen reader must not read a wall of empty boxes — unless it is given a name, which turns it into a busy progress indicator |
//! | Reduced motion | the shimmer loops forever and carries no information, so reduced motion **stops** it and leaves a plain block |
//!
//! ```text
//! cargo run -p silka-gallery -- --page skeleton
//! ```

use silka_core::app::{component, BuildCtx};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{row, View};
use silka_theme::{RadiusToken, Theme};
use silka_widgets::{
    button, card_padded, skeleton, skeleton_circle, skeleton_text, text, CardVariant,
};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Skeleton";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A spinner says \"something is happening\"; a \
    skeleton says \"THIS is happening, and this is the shape it will take\". \
    That is what keeps the page from jumping when the data lands — the room \
    was booked from the start.";

/// The button that swaps the placeholder for the real thing.
pub const TOMBOL_MUAT: &str = "Load";
/// The button that puts the placeholder back.
pub const TOMBOL_ULANG: &str = "Clear again";
/// The a11y name of the one skeleton that is deliberately announced.
pub const NAMA_SIBUK: &str = "Loading the summary";
/// The heading of the card that arrives once loaded.
pub const JUDUL_KARTU: &str = "August summary";
/// The body of the card that arrives once loaded.
pub const ISI_KARTU: &str = "Rp 128.400.000 from 42 transactions.";

/// The diameter of the round placeholder, in points.
pub const GARIS_TENGAH: f32 = 40.0;

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    let dimuat = use_signal(|| false);

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [bentuk(&t), tukar(dimuat), kendali(&t, dimuat)],
    )
}

/// The three shapes a skeleton comes in.
fn bentuk(t: &Theme) -> View {
    kepala::spesimen(
        t,
        "Lines, circles, blocks",
        [
            View::from(
                row([
                    View::from(skeleton_circle(GARIS_TENGAH)),
                    View::from(skeleton_text(3)),
                ])
                .spacing(t.space(4.0))
                .cross(CrossAlign::Start),
            ),
            View::from(
                skeleton()
                    .width(t.space(60.0))
                    .height(t.space(3.0))
                    .rounded(RadiusToken::Sm)
                    // The one placeholder with a name: it is the page's own
                    // "still working" announcement, and the others stay silent
                    // so a screen reader does not read four empty boxes.
                    .label(NAMA_SIBUK),
            ),
        ],
    )
}

/// The same box, before and after — the point of the component.
fn tukar(dimuat: Signal<bool>) -> View {
    component("tukar-skeleton", move |cx| {
        let t = kepala::mulai(cx);
        let isi: Vec<View> = if dimuat.get() {
            vec![
                View::from(
                    text(JUDUL_KARTU)
                        .size(t.typography.headline.size)
                        .color(t.color.label)
                        .single_line(),
                ),
                View::from(
                    text(ISI_KARTU)
                        .size(t.typography.body.size)
                        .color(t.color.secondary_label),
                ),
            ]
        } else {
            vec![
                View::from(
                    skeleton()
                        .height(t.typography.headline.size)
                        .width_fraction(0.6),
                ),
                View::from(skeleton_text(2)),
            ]
        };

        kepala::spesimen(
            &t,
            "Before and after",
            [View::from(
                card_padded(isi)
                    .variant(CardVariant::Outlined)
                    .label(JUDUL_KARTU),
            )],
        )
    })
}

/// The two buttons that flip it.
fn kendali(t: &Theme, dimuat: Signal<bool>) -> View {
    row([
        View::from(button(TOMBOL_MUAT).on_press(move || dimuat.set(true))),
        View::from(button(TOMBOL_ULANG).on_press(move || dimuat.set(false))),
    ])
    .spacing(t.space(3.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(820.0, 720.0);

    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn klik(ui: &mut AppRuntime, titik: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
    }

    #[test]
    fn hanya_satu_placeholder_yang_dibacakan() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        assert!(
            pohon.find_label(NAMA_SIBUK).is_some(),
            "placeholder bernama harus terbaca:\n{}",
            pohon.dump()
        );
        // The unnamed ones are hidden: a wall of empty boxes is noise, not
        // information.
        let jumlah = pohon
            .dump()
            .lines()
            .filter(|l| l.trim_start().starts_with("progress "))
            .count();
        assert_eq!(
            jumlah,
            1,
            "placeholder tanpa nama ikut dibacakan:\n{}",
            pohon.dump()
        );
    }

    #[test]
    fn kartu_menggantikan_placeholder_tanpa_menghilang() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        assert!(
            ui.access_tree().find_label(JUDUL_KARTU).is_some(),
            "kartu kosong pun tetap sebuah landmark bernama"
        );

        let p = kotak(&ui, TOMBOL_MUAT).center();
        klik(&mut ui, p);
        ui.frame();

        let pohon = ui.access_tree();
        assert!(
            pohon.dump().contains(ISI_KARTU),
            "isi sungguhan tidak pernah tiba:\n{}",
            pohon.dump()
        );
    }

    #[test]
    fn kilaunya_berhenti_saat_gerak_dikurangi() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        assert!(
            silka_widgets::skeleton::is_animating(ui.tree()),
            "tanpa pengaturan apa pun, placeholder berkilau"
        );

        let _ = ui.set_motion(Motion::Reduced);
        // The shimmer learns about the setting from the **tick**, so a frame
        // without one would still be sweeping.
        ui.animate(silka_widgets::advance);
        ui.frame();
        assert!(
            !silka_widgets::skeleton::is_animating(ui.tree()),
            "kilau adalah gerak dekoratif yang berulang selamanya: harus berhenti"
        );
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
