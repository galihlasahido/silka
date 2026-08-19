//! Demo page: **card** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | A vocabulary for the surface | four variants side by side: raised off the page, drawn on it, filled, or nothing but padding — a nested card that looked like its parent was the old bug |
//! | Header, body, footer | one card assembled from the three parts, with the hairline the header owns rather than a divider written at the call site |
//! | A card is a landmark | it carries a name, so a screen reader can jump to it; the title inside is then structural instead of a second announcement |
//! | A pressable card is a button | the last one takes Space/Enter, shows a focus ring, and says `button` — a clickable card that stayed a `group` would be unreachable from the keyboard |
//! | Correct in both presets | corners, hairline, elevation and padding are all tokens; `--preset tailwind` changes the corner *shape*, not the code |
//!
//! ```text
//! cargo run -p silka-gallery -- --page card
//! ```

use silka_core::app::{component, BuildCtx};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{card, card_body, card_footer, card_header, card_padded, text, CardVariant};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Card";

/// The paragraph under the title.
pub const KETERANGAN: &str = "The surface a group of content sits on. Both \
    applications in this repository grew one of their own at some point — the \
    same four lines, each with its own idea of how much padding a card \
    has.";

/// The name of the card assembled from header, body and footer.
pub const NAMA_FAKTUR: &str = "Latest invoice";
/// The subtitle of that card.
pub const SUB_FAKTUR: &str = "Last 30 days";
/// The name of the pressable card.
pub const NAMA_TEKAN: &str = "Open the recap";
/// What the page says before the pressable card has been used.
pub const BELUM: &str = "not pressed yet";

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    let ditekan = use_signal(|| 0u32);

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [varian(&t), bagian(&t), dapat_ditekan(ditekan)],
    )
}

/// The four surfaces, in a row so a nested one cannot hide.
fn varian(t: &Theme) -> View {
    let kartu = CardVariant::ALL.map(|v| {
        View::from(
            card_padded([View::from(
                text(v.name())
                    .size(t.typography.callout.size)
                    .color(t.color.label)
                    .single_line(),
            )])
            .key(v.name())
            .variant(v)
            .label(format!("Card {}", v.name())),
        )
    });

    kepala::spesimen(
        t,
        "Four surfaces",
        [View::from(
            row(kartu)
                .spacing(t.space(4.0))
                .cross(CrossAlign::Stretch)
                .wrap(),
        )],
    )
}

/// One card out of its three parts.
fn bagian(t: &Theme) -> View {
    let isi = card_body([
        View::from(
            text("Rp 128.400.000")
                .size(t.typography.title2.size)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text("42 transactions, 3 of them awaiting approval.")
                .size(t.typography.body.size)
                .color(t.color.secondary_label),
        ),
    ]);

    let kaki = card_footer([View::from(
        text("Updated 5 minutes ago")
            .size(t.typography.footnote.size)
            .color(t.color.tertiary_label)
            .single_line(),
    )]);

    kepala::spesimen(
        t,
        "Header, body, footer",
        [View::from(
            card([
                View::from(card_header(NAMA_FAKTUR).subtitle(SUB_FAKTUR)),
                View::from(isi),
                View::from(kaki),
            ])
            .label(NAMA_FAKTUR),
        )],
    )
}

/// The card that is a button — its own component, because it is the only place
/// the counter is read (§2.5).
fn dapat_ditekan(ditekan: Signal<u32>) -> View {
    component("kartu-ditekan", move |cx| {
        let t = kepala::mulai(cx);
        let n = ditekan.get();
        let keterangan = if n == 0 {
            BELUM.to_string()
        } else {
            format!("pressed {n}×")
        };

        kepala::spesimen(
            &t,
            "A pressable card",
            [View::from(
                column([
                    View::from(
                        card_padded([View::from(
                            text("This month's recap")
                                .size(t.typography.headline.size)
                                .color(t.color.label)
                                .single_line(),
                        )])
                        .variant(CardVariant::Elevated)
                        .label(NAMA_TEKAN)
                        .on_press(move || ditekan.update(|v| *v += 1)),
                    ),
                    kepala::catatan(&t, keterangan),
                ])
                .spacing(t.space(3.0))
                .cross(CrossAlign::Start),
            )],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(960.0, 820.0);

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
    fn kartu_adalah_landmark_dan_yang_bisa_ditekan_adalah_tombol() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        let faktur = pohon
            .find_label(NAMA_FAKTUR)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert!(
            !faktur.node.actions.contains(AccessActions::CLICK),
            "kartu biasa bukan kontrol"
        );

        let tekan = pohon
            .find_label(NAMA_TEKAN)
            .expect("kartu yang bisa ditekan");
        assert_eq!(tekan.node.role, AccessRole::Button);
        assert!(tekan.node.actions.contains(AccessActions::CLICK));
        assert!(tekan.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn menekan_kartu_menjalankan_aksinya_lewat_tetikus_dan_papan_ketik() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        assert!(ui.access_tree().dump().contains(BELUM));

        let p = kotak(&ui, NAMA_TEKAN).center();
        klik(&mut ui, p);
        ui.frame();
        assert!(
            ui.access_tree().dump().contains("pressed 1×"),
            "klik tidak menjalankan aksinya:\n{}",
            ui.access_tree().dump()
        );

        // The keyboard reaches it too — that is the whole point of it being a
        // button rather than a clickable box.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(80),
        )));
        ui.frame();
        assert!(ui.access_tree().dump().contains("pressed 2×"));
    }

    #[test]
    fn setiap_varian_muncul_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);
                let pohon = ui.access_tree();
                for v in CardVariant::ALL {
                    assert!(
                        pohon.find_label(&format!("Card {}", v.name())).is_some(),
                        "varian {} hilang di {preset:?}/{appearance:?}",
                        v.name()
                    );
                }
            }
        }
    }
}
