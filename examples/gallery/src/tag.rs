//! Demo page: **tag** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | A tag **does** something, a badge **says** something | the chips here are buttons carrying a toggled state; the pills on the badge page are labels |
//! | Interactive states on a spring | hover and press: background, border and focus ring all move, none of them cut |
//! | Keyboard | Tab to a chip, Space toggles it; Tab again reaches its **cross**, which is its own control; Delete on the chip removes it |
//! | The cross has its own name | a screen reader says "Delete Urgent", not "×" |
//! | Hit target ≥ 44pt | the chip's box clears the floor while the drawn pill stays small inside it |
//! | Correct in both presets | the tone comes from [`silka_widgets::BadgeTone`], the radius from [`silka_theme::RadiusToken::Full`] |
//!
//! ```text
//! cargo run -p silka-gallery -- --page tag
//! ```

use silka_core::app::{component, BuildCtx};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{row, View};
use silka_theme::Theme;
use silka_widgets::{button_variant, tag, BadgeTone, ButtonVariant};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Tag";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A pill you can press: a filter that lights up, and \
    a label you can take off. Its shape is nearly identical to a badge, and \
    that is the trap — what separates them is not decoration but the contract: \
    role, tab stop, touch target, and spring.";

/// The filter chips, in order.
pub const PENYARING: [&str; 4] = ["All", "Unpaid", "Due date", "Paid"];
/// The removable labels the page starts with.
pub const LABEL: [&str; 3] = ["Urgent", "Long-standing client", "Birthday"];
/// The chip that is deliberately unusable.
pub const TERKUNCI: &str = "Archive (locked)";
/// The prefix of a cross's accessible name.
pub const HAPUS: &str = "Delete";

/// The name a cross announces itself with.
///
/// Pure, and the one string this page owns: a cross that announced "×" would
/// be three identical buttons to a screen reader.
pub fn nama_hapus(label: &str) -> String {
    format!("{HAPUS} {label}")
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    let terpilih = use_signal(|| 0usize);
    let label = use_signal(|| LABEL.map(String::from).to_vec());

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [penyaring(terpilih), dapat_dicopot(label), mati(&t)],
    )
}

/// The filter row: exactly one chip on at a time.
fn penyaring(terpilih: Signal<usize>) -> View {
    component("penyaring-tag", move |cx| {
        let t = kepala::mulai(cx);
        let aktif = terpilih.get();
        let chip = PENYARING.iter().enumerate().map(|(i, nama)| {
            View::from(
                tag(*nama)
                    .key(*nama)
                    .tone(BadgeTone::Accent)
                    .selected(i == aktif)
                    .on_select(move |_| terpilih.set(i)),
            )
        });

        kepala::spesimen(
            &t,
            "Filters",
            [
                View::from(
                    row(chip.collect::<Vec<_>>())
                        .spacing(t.space(2.0))
                        .cross(CrossAlign::Center)
                        .wrap(),
                ),
                kepala::catatan(&t, format!("Active: {}", PENYARING[aktif])),
            ],
        )
    })
}

/// The removable labels, plus the button that brings them back.
fn dapat_dicopot(label: Signal<Vec<String>>) -> View {
    component("label-tag", move |cx| {
        let t = kepala::mulai(cx);
        let sekarang = label.get();

        let chip: Vec<View> = sekarang
            .iter()
            .map(|nama| {
                let untuk_hapus = nama.clone();
                View::from(
                    tag(nama.clone())
                        .key(nama.clone())
                        .tone(BadgeTone::Warning)
                        .remove_label(nama_hapus(nama))
                        .on_remove(move || {
                            label.update(|l| l.retain(|x| *x != untuk_hapus));
                        }),
                )
            })
            .collect();

        let isi: Vec<View> = if chip.is_empty() {
            vec![kepala::catatan(&t, "Every label has been taken off.")]
        } else {
            vec![View::from(
                row(chip)
                    .spacing(t.space(2.0))
                    .cross(CrossAlign::Center)
                    .wrap(),
            )]
        };

        let mut anak = isi;
        anak.push(View::from(
            button_variant("Bring them all back", ButtonVariant::Secondary)
                .on_press(move || label.set(LABEL.map(String::from).to_vec())),
        ));

        kepala::spesimen(&t, "Removable", anak)
    })
}

/// The chip that cannot be used — still announced, dimmed rather than hidden.
fn mati(t: &Theme) -> View {
    kepala::spesimen(
        t,
        "Unavailable",
        [View::from(
            row([View::from(
                tag(TERKUNCI)
                    .disabled(true)
                    .selected(true)
                    .on_select(|_| {}),
            )])
            .main(MainAlign::Start)
            .cross(CrossAlign::Center),
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessToggled};
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(880.0, 720.0);

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
    fn nama_silang_menyebut_apa_yang_dihapus() {
        assert_eq!(nama_hapus("Urgent"), "Delete Urgent");
    }

    #[test]
    fn chip_adalah_tombol_bertoggle_dengan_target_sentuh_yang_cukup() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        let e = pohon
            .find_label(PENYARING[0])
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        assert_eq!(
            e.node.toggled,
            Some(AccessToggled::On),
            "yang aktif menyala"
        );
        assert!(e.node.actions.contains(AccessActions::FOCUS));
        assert!(
            e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
            "target sentuh chip cuma {:?}",
            e.bounds.size
        );

        // The disabled one is still announced — dimmed, not hidden (§3.8).
        let terkunci = pohon.find_label(TERKUNCI).expect("tetap dibacakan");
        assert!(terkunci.node.disabled);
        assert!(!terkunci.node.actions.contains(AccessActions::CLICK));
    }

    #[test]
    fn memilih_chip_lain_memindahkan_keadaan_menyala() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();

        let p = kotak(&ui, PENYARING[2]).center();
        klik(&mut ui, p);
        ui.frame();

        let pohon = ui.access_tree();
        assert_eq!(
            pohon.find_label(PENYARING[2]).unwrap().node.toggled,
            Some(AccessToggled::On)
        );
        assert_eq!(
            pohon.find_label(PENYARING[0]).unwrap().node.toggled,
            Some(AccessToggled::Off),
            "penyaring adalah pilihan tunggal"
        );
    }

    #[test]
    fn silang_benar_benar_mencopot_labelnya() {
        let mut ui = ui(Theme::tailwind(Appearance::Dark));
        ui.frame();

        let p = kotak(&ui, &nama_hapus(LABEL[1])).center();
        klik(&mut ui, p);
        ui.frame();

        let pohon = ui.access_tree();
        assert!(
            pohon.find_label(LABEL[1]).is_none(),
            "label tidak jadi dicopot:\n{}",
            pohon.dump()
        );
        assert!(
            pohon.find_label(LABEL[0]).is_some(),
            "tetangganya ikut hilang"
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
