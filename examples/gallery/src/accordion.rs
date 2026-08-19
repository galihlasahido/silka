//! Demo page: **accordion / collapsible** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The hard part is the height, not the chevron | open a section: the rows **below** it slide down, and the paragraph inside does not re-wrap on the way — it is laid out at its natural height and clipped to the animation's |
//! | One at a time, or many | the first group is [`AccordionMode::Single`] and the second [`AccordionMode::Multiple`]; both come from one pure function, [`silka_widgets::toggled_set`] |
//! | Nothing open is a legitimate state | click an open header to close it, even in single mode — refusing that traps the reader in a section |
//! | A closed panel is **gone** | Tab past a closed section: the button inside it is unreachable, because closed means hidden from the a11y tree and skipped by focus, not merely unpainted |
//! | A disabled section stays shut | and still announces itself as dimmed rather than silently ignoring the click |
//! | AccessKit node | each header is a button carrying `expanded`; the group carries the accordion's name |
//!
//! ```text
//! cargo run -p silka-gallery -- --page accordion
//! ```

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, View};
use silka_theme::Theme;
use silka_widgets::accordion::AccordionMode;
use silka_widgets::{
    accordion, button_variant, collapsible, text, toggled_set, ButtonVariant, CardVariant,
};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Accordion";

/// The paragraph under the title.
pub const KETERANGAN: &str = "Content you can fold away. The hard part is not \
    the chevron but the height: the body is laid out at its natural height \
    (otherwise the paragraph re-wraps every frame and the text \"boils\"), \
    the box is as tall as the spring says (so the rows below move with it), \
    and the box clips (so the part with no room yet does not paint over its \
    neighbours).";

/// The a11y name of the single-open group.
pub const NAMA_TANYA: &str = "Frequently asked questions";
/// The a11y name of the multi-open group.
pub const NAMA_SETELAN: &str = "Advanced settings";

/// The headers of the single-open group.
pub const TANYA: [&str; 3] = [
    "How long does delivery take?",
    "How do I return an item?",
    "Can I pay on delivery?",
];

/// The answers, in the same order as [`TANYA`].
pub const JAWAB: [&str; 3] = [
    "Two to five working days on Java, and up to ten days elsewhere. The \
     tracking number is emailed as soon as the parcel is handed to the \
     courier.",
    "Thirty days, no questions asked. The item must still be in its original \
     packaging, and we cover the return shipping.",
    "Yes, for orders under two million rupiah and only in cities our partner \
     couriers serve.",
];

/// The headers of the multi-open group.
pub const SETELAN: [&str; 3] = ["Network", "Storage", "Diagnostics (locked)"];

/// The button that lives inside the first answer — what proves a closed panel
/// is genuinely out of the Tab order.
pub const TOMBOL_LACAK: &str = "Track a parcel";

/// The summary line's prefix, so a test can read the open set without pixels.
pub const AWALAN_TERBUKA: &str = "Open: ";

/// The section index that is deliberately disabled.
pub const INDEKS_TERKUNCI: usize = 2;

/// How the open set reads on the summary line.
pub fn ringkas(terbuka: &[usize]) -> String {
    if terbuka.is_empty() {
        return format!("{AWALAN_TERBUKA}none");
    }
    let nomor: Vec<String> = terbuka.iter().map(|i| (i + 1).to_string()).collect();
    format!("{AWALAN_TERBUKA}{}", nomor.join(", "))
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let tanya = use_signal(|| vec![0usize]);
    let setelan = use_signal(Vec::<usize>::new);

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [
            kepala::spesimen(
                &t,
                "One at a time",
                [
                    grup_tanya(&t, tanya),
                    kepala::catatan(&t, ringkas(&tanya.get())),
                ],
            ),
            kepala::spesimen(
                &t,
                "As many as you like",
                [
                    grup_setelan(&t, setelan),
                    kepala::catatan(
                        &t,
                        "The third section is disabled on purpose: it is still \
                         announced as dimmed rather than silently swallowing \
                         the click.",
                    ),
                ],
            ),
        ],
    )
}

/// The single-open group: an FAQ, which is what this mode exists for.
fn grup_tanya(t: &Theme, terbuka: Signal<Vec<usize>>) -> View {
    let sekarang = terbuka.get();
    let bagian = TANYA.iter().enumerate().map(|(i, judul)| {
        let isi: View = if i == 0 {
            column([
                kepala::catatan(t, JAWAB[i]),
                View::from(button_variant(TOMBOL_LACAK, ButtonVariant::Secondary)),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Start)
            .into()
        } else {
            kepala::catatan(t, JAWAB[i])
        };
        collapsible(*judul)
            .key(i as i64)
            .content(isi)
            .open(sekarang.contains(&i))
            .on_toggle(move |_| {
                terbuka.update(|v| *v = toggled_set(v, i, AccordionMode::Single));
            })
    });

    accordion(bagian).label(NAMA_TANYA).into()
}

/// The multi-open group: a settings panel, and one section that refuses.
fn grup_setelan(t: &Theme, terbuka: Signal<Vec<usize>>) -> View {
    let sekarang = terbuka.get();
    let bagian = SETELAN.iter().enumerate().map(|(i, judul)| {
        collapsible(*judul)
            .key(i as i64)
            .subtitle(if i == INDEKS_TERKUNCI {
                "Requires administrator rights"
            } else {
                "Available"
            })
            .content(
                text(format!("Body of the \"{judul}\" section."))
                    .size(t.typography.callout.size)
                    .color(t.color.secondary_label),
            )
            .disabled(i == INDEKS_TERKUNCI)
            .open(sekarang.contains(&i))
            .on_toggle(move |_| {
                terbuka.update(|v| *v = toggled_set(v, i, AccordionMode::Multiple));
            })
    });

    accordion(bagian)
        .variant(CardVariant::Outlined)
        .label(NAMA_SETELAN)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 900.0);
    const FRAME: Duration = Duration::from_millis(16);

    struct Uji {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Uji {
        fn baru(theme: Theme) -> Self {
            let ui = headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height);
            let mut uji = Self {
                ui,
                jam: Instant::now(),
            };
            uji.diam();
            uji
        }

        fn frame(&mut self) {
            self.jam += FRAME;
            self.ui.animate_at(self.jam, crate::shell::maju);
            self.ui.frame();
        }

        fn diam(&mut self) {
            let mut n = 0;
            while !self.ui.is_idle() {
                self.frame();
                n += 1;
                assert!(n < 600, "halaman tidak pernah diam");
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

        fn tombol(&mut self, label: &str) {
            let p = self.kotak(label).center();
            for e in [
                PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
                PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                    .button(PointerButton::Primary),
                PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                    .button(PointerButton::Primary),
            ] {
                self.ui.dispatch(&Event::Pointer(e));
            }
            self.diam();
        }

        fn terbuka(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with(AWALAN_TERBUKA))
                .unwrap_or_else(|| panic!("baris ringkasan hilang:\n{}", pohon.dump()))
        }

        fn titik(&self, label: &str) -> Point {
            self.kotak(label).center()
        }
    }

    #[test]
    fn ringkasan_menerjemahkan_indeks_ke_nomor_manusia() {
        assert_eq!(ringkas(&[]), "Open: none");
        assert_eq!(ringkas(&[0]), "Open: 1");
        assert_eq!(ringkas(&[0, 2]), "Open: 1, 3");
    }

    #[test]
    fn hanya_bagian_yang_terbuka_yang_isinya_terbaca() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        for judul in TANYA {
            assert!(uji.ada(judul), "{judul} hilang");
        }
        assert!(uji.ada(JAWAB[0]), "bagian pertama seharusnya terbuka");
        assert!(
            !uji.ada(JAWAB[1]),
            "isi bagian tertutup masih dibacakan — laci tertutup harus \
             benar-benar hilang, bukan sekadar tidak digambar"
        );
        assert!(uji.terbuka().ends_with("1"));
    }

    #[test]
    fn tombol_di_dalam_laci_tertutup_tidak_bisa_dijangkau() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        assert!(uji.ada(TOMBOL_LACAK), "laci pertama terbuka, tombolnya ada");

        uji.tombol(TANYA[0]);
        assert!(
            !uji.ada(TOMBOL_LACAK),
            "tombol di dalam laci tertutup masih bisa di-Tab: ring fokusnya \
             akan hilang ke dalam laci"
        );
        assert!(uji.terbuka().ends_with("none"));
    }

    #[test]
    fn satu_per_satu_menutup_yang_lain() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        uji.tombol(TANYA[1]);

        assert!(uji.ada(JAWAB[1]));
        assert!(
            !uji.ada(JAWAB[0]),
            "membuka yang kedua tidak menutup yang pertama"
        );
        assert!(uji.terbuka().ends_with("2"));
    }

    #[test]
    fn bagian_yang_dimatikan_tetap_tertutup_dan_tetap_diumumkan() {
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark));
        let terkunci = SETELAN[INDEKS_TERKUNCI];

        let e = uji
            .ui
            .access_tree()
            .find_label(terkunci)
            .expect("bagian terkunci hilang")
            .node
            .clone();
        assert_eq!(e.role, AccessRole::Button);
        assert!(e.disabled, "bagian terkunci tidak diumumkan sebagai redup");

        let sebelum = uji.titik(SETELAN[0]);
        uji.tombol(terkunci);
        assert!(
            !uji.ada("Body of the \"Diagnostics (locked)\" section."),
            "bagian yang dimatikan tetap terbuka"
        );
        // …and nothing else moved either.
        assert_eq!(uji.titik(SETELAN[0]), sebelum);
    }

    #[test]
    fn membuka_bagian_menggeser_baris_di_bawahnya() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        let sebelum = uji.kotak(SETELAN[1]).min_y();
        uji.tombol(SETELAN[0]);
        let sesudah = uji.kotak(SETELAN[1]).min_y();
        assert!(
            sesudah > sebelum,
            "baris di bawahnya tidak bergeser ({sebelum} → {sesudah}): isinya \
             menimpa tetangganya, bukan mendorongnya"
        );
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
