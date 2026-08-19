//! Demo page: **toast** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | It removes itself | press "Info" and wait — four seconds later it asks the page to drop it, and the page does |
//! | Hovering pauses the countdown | press one, then rest the pointer on it: it stays. A message that vanishes while you are reading it is worse than no message |
//! | Swipe to dismiss | drag a toast sideways and let go: the finger's velocity is handed to the spring, so a flick throws it out and a nudge springs it back |
//! | The stack has a ceiling | press "Info" five times: three are on screen and the rest wait, because [`silka_widgets::toast::stack_window`] says so |
//! | A sticky one waits for you | the error never expires; only its close button or a swipe removes it |
//! | The tone is not the colour | each toast carries an icon as well, because colour alone is never a status (§3.8) |
//! | AccessKit node | a named region holding one group per message, each carrying its whole text |
//!
//! ```text
//! cargo run -p silka-gallery -- --page toast
//! ```
//!
//! **A limitation stated rather than hidden:** `AccessNode` has no live-region
//! concept yet, so a toast is announced when a screen reader reaches it rather
//! than the moment it appears.

use silka_core::app::BuildCtx;
use silka_core::signals::use_signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{row, View};
use silka_theme::Theme;
use silka_widgets::toast::TOAST_STACK_MAX;
use silka_widgets::{
    button, button_variant, overlay_layer, toast, toasts, use_toast_state, ButtonVariant,
    ToastState, ToastTone,
};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Toast";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A fleeting message that stacks up in one corner, \
    counts itself down, and can be flicked away. The list belongs to the \
    application: this component never removes one by itself — it asks, and this \
    page decides.";

/// The name of the whole stack, as a screen reader announces it.
pub const NAMA_TUMPUKAN: &str = "Notifications";

/// The button that pushes an informational toast.
pub const TOMBOL_INFO: &str = "Info";
/// The button that pushes a success toast with an action.
pub const TOMBOL_SUKSES: &str = "Success + Undo";
/// The button that pushes the sticky error.
pub const TOMBOL_GALAT: &str = "Error (sticky)";
/// The button that empties the stack.
pub const TOMBOL_BERSIH: &str = "Clear";

/// What the informational toast says.
pub const TEKS_INFO: &str = "Sync running";
/// What the success toast says.
pub const TEKS_SUKSES: &str = "Invoice sent";
/// The success toast's action button.
pub const TOMBOL_URUNG: &str = "Undo";
/// What the sticky error says.
pub const TEKS_GALAT: &str = "Upload failed";
/// The second line of the sticky error.
pub const RINCI_GALAT: &str = "The file is larger than 25 MB.";

/// What a screen reader announces for the sticky error — title and detail as
/// one sentence, which is also what a test looks for.
pub fn ringkas_galat() -> String {
    format!("{TEKS_GALAT}. {RINCI_GALAT}")
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    // The list lives in a signal, like all state; `ToastState` is a thin
    // convenience over exactly that, not a privileged store.
    let daftar = use_toast_state();
    let diurungkan = use_signal(|| 0u32);

    let isi = kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [
            kepala::spesimen(
                &t,
                "Three tones, one stack",
                [
                    View::from(tombol(&t, daftar, diurungkan)),
                    kepala::catatan(
                        &t,
                        format!(
                            "Queued: {} · at most {TOAST_STACK_MAX} shown · \
                             undone {} times",
                            daftar.len(),
                            diurungkan.get()
                        ),
                    ),
                ],
            ),
            kepala::spesimen(
                &t,
                "What it does on its own",
                [kepala::catatan(
                    &t,
                    "It counts down, stops counting while the pointer is over \
                     it, takes a swipe at the finger's velocity, and animates \
                     out first — only then asking to be removed.",
                )],
            ),
        ],
    );

    overlay_layer(isi)
        .overlay(
            toasts(daftar.items())
                .label(NAMA_TUMPUKAN)
                .max(TOAST_STACK_MAX)
                .on_dismiss(move |id| {
                    daftar.dismiss(id);
                }),
        )
        .into()
}

/// The four buttons that drive the stack.
fn tombol(t: &Theme, daftar: ToastState, diurungkan: silka_core::signals::Signal<u32>) -> View {
    row([
        View::from(button(TOMBOL_INFO).on_press(move || {
            daftar.push(toast(TEKS_INFO).tone(ToastTone::Info));
        })),
        View::from(
            button_variant(TOMBOL_SUKSES, ButtonVariant::Secondary).on_press(move || {
                daftar.push(
                    toast(TEKS_SUKSES)
                        .tone(ToastTone::Success)
                        .action(TOMBOL_URUNG, move || diurungkan.update(|n| *n += 1)),
                );
            }),
        ),
        View::from(
            button_variant(TOMBOL_GALAT, ButtonVariant::Destructive).on_press(move || {
                daftar.push(
                    toast(TEKS_GALAT)
                        .description(RINCI_GALAT)
                        .tone(ToastTone::Error)
                        .sticky(),
                );
            }),
        ),
        View::from(
            button_variant(TOMBOL_BERSIH, ButtonVariant::Ghost).on_press(move || daftar.clear()),
        ),
    ])
    .spacing(t.space(3.0))
    .main(MainAlign::Start)
    .cross(CrossAlign::Center)
    .wrap()
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(1000.0, 720.0);
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

        /// How many nodes mention this text — how a stack of identical toasts
        /// is counted.
        fn berapa(&self, potongan: &str) -> usize {
            self.ui
                .access_tree()
                .entries()
                .iter()
                .filter(|e| {
                    e.node
                        .label
                        .as_deref()
                        .is_some_and(|l| l.contains(potongan))
                })
                .count()
        }

        fn klik(&mut self, titik: Point) {
            for e in [
                PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
                PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                    .button(PointerButton::Primary),
                PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                    .button(PointerButton::Primary),
            ] {
                self.ui.dispatch(&Event::Pointer(e));
            }
            self.frames(2);
        }

        fn tombol(&mut self, label: &str) {
            let p = self.kotak(label).center();
            self.klik(p);
        }

        /// Move the pointer somewhere harmless, so a toast's countdown is not
        /// paused by a cursor left over the corner it stacks in.
        fn menjauh(&mut self) {
            self.ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                Point::new(4.0, 4.0),
                Duration::ZERO,
            )));
            self.frame();
        }
    }

    #[test]
    fn halaman_dimulai_tanpa_pesan() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(!uji.ada(TEKS_INFO));
        for label in [TOMBOL_INFO, TOMBOL_SUKSES, TOMBOL_GALAT, TOMBOL_BERSIH] {
            assert!(uji.ada(label), "{label} hilang");
        }
    }

    #[test]
    fn pesan_muncul_lalu_menghapus_dirinya_sendiri() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        uji.tombol(TOMBOL_INFO);
        uji.menjauh();
        assert!(uji.ada(TEKS_INFO), "pesan tidak muncul");

        // Four seconds of countdown plus the exit transition, on a made-up
        // clock: a test must never wait on real time (§9.5).
        uji.frames(360);
        assert!(
            !uji.ada(TEKS_INFO),
            "pesan tidak pernah menghapus dirinya sendiri"
        );
    }

    #[test]
    fn yang_menetap_tidak_pergi_sendiri() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        uji.tombol(TOMBOL_GALAT);
        uji.menjauh();
        uji.frames(400);
        assert!(
            uji.ada(&ringkas_galat()),
            "pesan menetap ikut kedaluwarsa — padahal itu satu-satunya \
             pembedanya"
        );

        uji.tombol(TOMBOL_BERSIH);
        uji.frames(60);
        assert!(!uji.ada(&ringkas_galat()));
    }

    #[test]
    fn tumpukan_punya_langit_langit() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        for _ in 0..5 {
            uji.tombol(TOMBOL_INFO);
        }
        uji.menjauh();
        uji.frames(4);

        assert!(
            uji.berapa(TEKS_INFO) <= TOAST_STACK_MAX,
            "{} pesan tampil sekaligus, batasnya {TOAST_STACK_MAX}",
            uji.berapa(TEKS_INFO)
        );
        // …and the ones waiting are still in the application's list, which is
        // why the summary line counts five.
        assert!(
            uji.ada("Queued: 5 · at most 3 shown · undone 0 times"),
            "antrean tidak terhitung:\n{}",
            uji.ui.access_tree().dump()
        );
    }

    #[test]
    fn tombol_aksi_di_dalam_pesan_benar_benar_jalan() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        uji.tombol(TOMBOL_SUKSES);
        uji.menjauh();
        // The entrance comes from off-screen, so the action button is not
        // inside the window yet: clicking where it *will* be would miss.
        uji.frames(40);
        uji.tombol(TOMBOL_URUNG);
        uji.frames(4);

        assert!(
            uji.ada("Queued: 1 · at most 3 shown · undone 1 times"),
            "aksi di dalam pesan cuma gambar:\n{}",
            uji.ui.access_tree().dump()
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
