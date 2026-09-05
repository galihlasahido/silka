//! Demo page: **hover card** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | You are allowed to move into it | rest on the mention, wait for the card, then travel to it — the card stays, because it takes the pointer ([`Barrier::Panel`]) while a tooltip refuses to |
//! | It is slower than a tooltip, on purpose | a preview that flashes past while the pointer crosses a paragraph is noise, so it waits 700 ms and lingers 300 |
//! | It really is a popover | the anchor, the arrow, the auto-flip and the transition are [`mod@silka_widgets::popover`]'s, unchanged — three defaults are all that differ |
//! | Rich content, not one line | an avatar, a name, a line of prose and a button that works |
//! | AccessKit node | announced as a dialog carrying the person's name; the page behind it stays reachable |
//!
//! ```text
//! cargo run -p silka-gallery -- --page hover-card
//! ```
//!
//! # Two watched shapes, one panel
//!
//! [`crate::sentuh`] watches the mention **and** the card's own body, and the
//! panel is up while either is under the pointer. That is the whole difference
//! between this and [`crate::tooltip`]: travelling from the link to the card
//! crosses a gap, and a component that watched only the link would go away
//! halfway across it.

use silka_core::access::AccessRole;
use silka_core::app::BuildCtx;
use silka_core::signals::use_signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, interactive, row, View};
use silka_paint::Insets;
use silka_theme::Theme;
use silka_widgets::overlay::{Anchor, Barrier, Side};
use silka_widgets::tooltip::TooltipDelay;
use silka_widgets::{
    avatar, button_variant, hover_card, overlay_layer, text, ButtonVariant, HOVER_CARD_DELAY,
};

use crate::{jangkar, kepala, sentuh};

/// The page title.
pub const JUDUL: &str = "Hover card";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A preview that appears when the pointer rests on a \
    mention — and that you are allowed to move into. That is its one important \
    difference from a tooltip: the panel accepts the pointer, so walking over \
    to it still counts as \"still on it\", while outside the panel every click \
    passes straight through to the page.";

/// The node key of the mention that opens the card.
pub const KUNCI_SEBUTAN: &str = "hover-card-sebutan";
/// The node key of the card's own body — the second shape [`crate::sentuh`]
/// watches, so travelling into the card counts as staying.
pub const KUNCI_PANEL: &str = "hover-card-panel";

/// The mention's text, which is also its a11y name.
pub const SEBUTAN: &str = "@ada";

/// The person the card is about — also the card's a11y name.
pub const NAMA: &str = "Ada Lovelace";
/// The line of prose under the name.
pub const BIO: &str = "Wrote the first algorithm for a machine that was \
    never built.";
/// The button on the card.
pub const TOMBOL_IKUTI: &str = "Follow";

/// The sentence the mention sits in, so the card has something to hover *over*.
pub const KALIMAT: &str = "Meeting notes rewritten by";

/// The delays the card's own body uses.
///
/// Opening is instant: the pointer arriving on the panel means it is already
/// there, and a second wait would make the card blink out halfway through the
/// journey it exists to allow. Leaving keeps the card's own grace period.
pub const PANEL_DELAY: TooltipDelay = TooltipDelay {
    open: std::time::Duration::ZERO,
    close: HOVER_CARD_DELAY.close,
    warm: std::time::Duration::ZERO,
};

/// The barrier the card uses — restated here because it is the one property a
/// screenshot cannot show.
pub const PENGHALANG: Barrier = Barrier::Panel;

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let jangkar_sebutan = use_signal(Anchor::default);
    let atas_sebutan = use_signal(|| false);
    let atas_panel = use_signal(|| false);
    jangkar::lacak(KUNCI_SEBUTAN, jangkar_sebutan);
    sentuh::lacak(KUNCI_SEBUTAN, atas_sebutan, HOVER_CARD_DELAY);
    sentuh::lacak(KUNCI_PANEL, atas_panel, PANEL_DELAY);

    // Either shape under the pointer keeps the card up — the "travel to it"
    // rule, written once.
    let terbuka = atas_sebutan.get() || atas_panel.get();

    let isi = kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [kepala::spesimen(
            &t,
            "A mention inside a sentence",
            [
                View::from(
                    row([kepala::catatan(&t, KALIMAT), sebutan(&t)])
                        .spacing(t.space(1.5))
                        .cross(CrossAlign::Center),
                ),
                kepala::catatan(
                    &t,
                    "Rest the pointer on that mention, wait a moment, then \
                     walk over to the card: the card stays.",
                ),
            ],
        )],
    );

    overlay_layer(isi)
        .overlay(
            hover_card(kartu(&t))
                .key("kartu")
                .open(terbuka)
                .anchor(jangkar_sebutan.get())
                .side(Side::Bottom)
                .width(t.space(66.0))
                .label(NAMA),
        )
        .into()
}

/// The mention itself: a hoverable shape that is **not** a button, because it
/// does nothing when pressed.
fn sebutan(t: &Theme) -> View {
    interactive(
        text(SEBUTAN)
            .size(t.typography.callout.size)
            .color(t.color.accent)
            .single_line(),
    )
    .key(KUNCI_SEBUTAN)
    .role(AccessRole::Label)
    .label(SEBUTAN)
    .focusable(false)
    .rounded_sm()
    .hover_bg(silka_theme::ColorToken::SurfaceHover)
    .into()
}

/// The card's body — keyed, so [`crate::sentuh`] can see the pointer arrive on
/// it and keep the panel up.
fn kartu(t: &Theme) -> View {
    interactive(
        column([
            View::from(
                row([
                    View::from(avatar(NAMA).sm()),
                    View::from(
                        text(NAMA)
                            .size(t.typography.headline.size)
                            .color(t.color.label)
                            .single_line(),
                    ),
                ])
                .spacing(t.space(2.5))
                .cross(CrossAlign::Center),
            ),
            kepala::catatan(t, BIO),
            View::from(button_variant(TOMBOL_IKUTI, ButtonVariant::Secondary)),
        ])
        .spacing(t.space(2.5))
        .cross(CrossAlign::Start)
        .main(MainAlign::Start)
        .padding(Insets::all(t.space(0.5))),
    )
    .key(KUNCI_PANEL)
    .role(AccessRole::Group)
    .focusable(false)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 700.0);
    const FRAME: Duration = Duration::from_millis(16);

    struct Uji {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Uji {
        fn baru(theme: Theme) -> Self {
            jangkar::lupakan();
            sentuh::lupakan();
            let ui = headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height);
            let mut uji = Self {
                ui,
                jam: Instant::now(),
            };
            uji.frame();
            uji
        }

        fn frame(&mut self) {
            self.jam += FRAME;
            self.ui.animate_at(self.jam, crate::shell::maju);
            self.ui.frame();
        }

        fn frames(&mut self, n: usize) {
            for _ in 0..n {
                self.frame();
            }
        }

        fn kotak(&self, label: &str) -> Rect {
            let pohon = self.ui.access_tree();
            pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
                .bounds
        }

        fn ada(&self, label: &str) -> bool {
            self.ui.access_tree().find_label(label).is_some()
        }

        fn arahkan(&mut self, titik: Point) {
            self.ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                titik,
                Duration::ZERO,
            )));
            self.frame();
        }
    }

    #[test]
    fn halaman_dimulai_tanpa_kartu() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(uji.ada(SEBUTAN), "sebutannya hilang");
        assert!(!uji.ada(BIO), "kartu muncul tanpa pointer");
    }

    #[test]
    fn menunggu_lebih_lama_daripada_tooltip_lalu_muncul() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        let p = uji.kotak(SEBUTAN).center();
        uji.arahkan(p);

        // A tooltip would already be up by here; a preview must not be.
        uji.frames(31);
        assert!(
            !uji.ada(BIO),
            "kartu muncul secepat tooltip — jeda 700 ms-nya hilang"
        );

        uji.frames(30);
        assert!(uji.ada(BIO), "kartu tidak pernah muncul");
        assert!(uji.ada(TOMBOL_IKUTI), "tombol di dalam kartu hilang");
    }

    #[test]
    fn berjalan_ke_kartunya_tidak_menutupnya() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        uji.arahkan(uji.kotak(SEBUTAN).center());
        uji.frames(60);
        assert!(uji.ada(BIO));

        // Straight onto the card — measured on its prose, because the card's
        // **name** belongs to the overlay entry, which spans the whole layer.
        // This is the whole component: a tooltip would be gone the moment the
        // pointer left its trigger.
        let kartu = uji.kotak(BIO).center();
        uji.arahkan(kartu);
        uji.frames(60);
        assert!(
            uji.ada(BIO),
            "kartu hilang saat pointer berpindah ke atasnya"
        );

        // Away from both, and it finally leaves.
        uji.arahkan(Point::new(VIEWPORT.width - 4.0, 4.0));
        uji.frames(90);
        assert!(!uji.ada(BIO), "kartu bertahan setelah pointer pergi");
    }

    #[test]
    fn halaman_terbangun_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let uji = Uji::baru(t);
                assert_eq!(uji.ui.scene().clear_color(), t.color.background);
                assert!(
                    !uji.ui.scene().is_empty(),
                    "{preset:?}/{appearance:?}: kosong"
                );
            }
        }
    }
}
