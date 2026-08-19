//! Demo page: **tooltip** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | It waits, and the wait is the component | rest the pointer on "Save" — nothing happens for half a second, and *that* is why crossing a toolbar leaves no trail of panels behind |
//! | The grace period | move off the trigger and straight back: the panel never blinks, because leaving starts a countdown rather than a close |
//! | The impatient variant | the second trigger uses [`silka_widgets::tooltip::TooltipDelay::instant`] — the same component, a different pair of numbers |
//! | It never catches the pointer | move *onto* the panel: it disappears, because a tooltip is [`silka_widgets::overlay::Barrier::None`] and the control under it stopped being hovered |
//! | Auto-flip | drag the window until a trigger nears the screen edge; the panel changes sides without this page computing a single coordinate |
//! | AccessKit node | announced as a tooltip carrying its own text — the control it describes keeps its own name |
//! | Reduced motion | the fade is **decorative**, so reduced motion removes it outright instead of merely calming it |
//!
//! ```text
//! cargo run -p silka-gallery -- --page tooltip
//! ```
//!
//! # Where "the pointer is here" comes from
//!
//! Not from the widget crate: [`silka_widgets::tooltip::TooltipTimer`] is
//! deliberately pure, and there is no "pointer entered widget X" hook in the
//! frame cycle yet. The gallery supplies both halves itself —
//! [`crate::jangkar`] publishes *where* the trigger is and [`crate::sentuh`]
//! publishes *whether it is hovered* — and both are read here as ordinary
//! signals.

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{row, View};
use silka_widgets::overlay::{Anchor, Side};
use silka_widgets::tooltip::TooltipDelay;
use silka_widgets::{button, button_variant, overlay_layer, tooltip, ButtonVariant};

use crate::{jangkar, kepala, sentuh};

/// The page title.
pub const JUDUL: &str = "Tooltip";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A short label that appears beside whatever the \
    pointer rests on. What makes it a component is not the panel but the wait: \
    half a second before it appears, a tenth of a second before it leaves, and \
    one \"warm\" second when a neighbour is the next thing rested on.";

/// The node key of the patient trigger — what [`crate::jangkar`] measures and
/// [`crate::sentuh`] watches.
pub const KUNCI_SABAR: &str = "tooltip-sabar";
/// The node key of the instant trigger.
pub const KUNCI_CEPAT: &str = "tooltip-cepat";
/// The node key of the trigger whose panel sits on the reading-end side.
pub const KUNCI_SAMPING: &str = "tooltip-samping";

/// The patient trigger's own name.
pub const TOMBOL_SABAR: &str = "Save";
/// The instant trigger's own name.
pub const TOMBOL_CEPAT: &str = "Duplicate";
/// The side-placed trigger's own name.
pub const TOMBOL_SAMPING: &str = "Archive it";

/// What the patient tooltip says.
pub const TEKS_SABAR: &str = "Save to file (⌘S)";
/// What the instant tooltip says.
pub const TEKS_CEPAT: &str = "Copy as a new document";
/// What the side-placed tooltip says.
pub const TEKS_SAMPING: &str = "Move to the archive";

/// The delays the first trigger uses — the platform default.
pub const SABAR: TooltipDelay = TooltipDelay::HIG;

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    // Three triggers, three pairs of signals. Registering on every rebuild is
    // free: both seams replace an entry rather than appending one, and the
    // hover timer survives so the countdown is not restarted sixty times a
    // second.
    let sabar = pemicu(KUNCI_SABAR, SABAR);
    let cepat = pemicu(KUNCI_CEPAT, TooltipDelay::instant());
    let samping = pemicu(KUNCI_SAMPING, SABAR);

    let isi = kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [
            kepala::spesimen(
                &t,
                "It waits, then it answers",
                [
                    View::from(
                        row([
                            View::from(button(TOMBOL_SABAR).key(KUNCI_SABAR)),
                            View::from(
                                button_variant(TOMBOL_CEPAT, ButtonVariant::Secondary)
                                    .key(KUNCI_CEPAT),
                            ),
                        ])
                        .spacing(t.space(3.0))
                        .cross(CrossAlign::Center),
                    ),
                    kepala::catatan(
                        &t,
                        "The left one waits half a second; the right one answers \
                         instantly. Same component — the only difference is two \
                         numbers.",
                    ),
                ],
            ),
            kepala::spesimen(
                &t,
                "A logical side, not left-right",
                [
                    View::from(
                        button_variant(TOMBOL_SAMPING, ButtonVariant::Ghost).key(KUNCI_SAMPING),
                    ),
                    kepala::catatan(
                        &t,
                        "The panel is asked for on the trailing side: right in \
                         an LTR document, left in an RTL one — and it still \
                         flips on its own when the window edge is close.",
                    ),
                ],
            ),
        ],
    );

    // Content first, panels after: the order written here **is** the stacking
    // order, and not one panel computes its own position.
    overlay_layer(isi)
        .overlay(
            tooltip(TEKS_SABAR)
                .key("panel-sabar")
                .open(sabar.terbuka.get())
                .anchor(sabar.jangkar.get())
                .side(Side::Top),
        )
        .overlay(
            tooltip(TEKS_CEPAT)
                .key("panel-cepat")
                .open(cepat.terbuka.get())
                .anchor(cepat.jangkar.get())
                .side(Side::Bottom),
        )
        .overlay(
            tooltip(TEKS_SAMPING)
                .key("panel-samping")
                .open(samping.terbuka.get())
                .anchor(samping.jangkar.get())
                .side(Side::End),
        )
        .into()
}

/// The two signals one trigger needs: where it is, and whether it is hovered.
#[derive(Clone, Copy)]
struct Pemicu {
    jangkar: Signal<Anchor>,
    terbuka: Signal<bool>,
}

/// Register a trigger with both gallery seams and hand back its signals.
fn pemicu(kunci: &'static str, delay: TooltipDelay) -> Pemicu {
    let p = Pemicu {
        jangkar: use_signal(Anchor::default),
        terbuka: use_signal(|| false),
    };
    jangkar::lacak(kunci, p.jangkar);
    sentuh::lacak(kunci, p.terbuka, delay);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset, Theme};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 700.0);
    /// One 60 Hz frame — a made-up clock, because a test must never wait on
    /// real time to let a countdown run (§9.5).
    const FRAME: Duration = Duration::from_millis(16);

    /// This page inside the same lifecycle the shell runs it in: **`maju`**,
    /// not `silka_widgets::advance`, because the two gallery seams this page
    /// depends on live there.
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

        /// Put the pointer at `titik` and deliver the frame it schedules.
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
    fn halaman_dimulai_tanpa_satu_pun_panel() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        for teks in [TEKS_SABAR, TEKS_CEPAT, TEKS_SAMPING] {
            assert!(!uji.ada(teks), "{teks} muncul tanpa pointer");
        }
        for tombol in [TOMBOL_SABAR, TOMBOL_CEPAT, TOMBOL_SAMPING] {
            assert!(uji.ada(tombol), "{tombol} hilang");
        }
    }

    #[test]
    fn pointer_yang_diam_membuka_panel_tapi_tidak_seketika() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        let p = uji.kotak(TOMBOL_SABAR).center();

        uji.arahkan(p);
        assert!(
            !uji.ada(TEKS_SABAR),
            "muncul di frame pertama: penantiannya hilang, dan penantian itulah \
             komponennya"
        );

        // The pointer is not moving any more, so nothing but the seam's own
        // request for the next frame can finish the countdown.
        uji.frames(60);
        let pohon = uji.ui.access_tree();
        let panel = pohon
            .find_label(TEKS_SABAR)
            .unwrap_or_else(|| panic!("panel tidak muncul:\n{}", pohon.dump()));
        assert_eq!(panel.node.role, AccessRole::Tooltip);

        // …and it really is beside the trigger, not in the middle of the layer,
        // which is where an **unanchored** overlay lands. The entry itself
        // spans the whole layer (that is what makes it a layer), so what is
        // measured is the panel it drew inside itself.
        let pemicu = uji.kotak(TOMBOL_SABAR);
        let dalam = *panel
            .children
            .first()
            .unwrap_or_else(|| panic!("entri tooltip kosong:\n{}", pohon.dump()));
        let kotak = pohon.get(dalam).expect("panel tergambar").bounds;
        assert!(
            kotak.max_y() <= pemicu.min_y(),
            "panel {kotak:?} tidak berada di atas pemicunya {pemicu:?}"
        );
        assert!(
            kotak.size.width < VIEWPORT.width,
            "panel selebar layar: jangkarnya tidak terbaca"
        );
    }

    #[test]
    fn yang_seketika_tidak_menunggu_sama_sekali() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        let p = uji.kotak(TOMBOL_CEPAT).center();

        uji.arahkan(p);
        // One more frame: the seam reads the hover flag *after* the layout that
        // the pointer event scheduled.
        uji.frame();
        assert!(uji.ada(TEKS_CEPAT), "delay nol masih menunggu");
        assert!(
            !uji.ada(TEKS_SABAR),
            "panel tetangga ikut muncul: hover-nya bocor antar pemicu"
        );
    }

    #[test]
    fn menjauh_menutup_panelnya_lagi() {
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark));
        let p = uji.kotak(TOMBOL_CEPAT).center();
        uji.arahkan(p);
        uji.frames(4);
        assert!(uji.ada(TEKS_CEPAT));

        uji.arahkan(Point::new(VIEWPORT.width - 4.0, VIEWPORT.height - 4.0));
        uji.frames(90);
        assert!(!uji.ada(TEKS_CEPAT), "panel bertahan setelah pointer pergi");
    }

    #[test]
    fn halaman_terbangun_dan_diam_di_kedua_preset() {
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
