//! Demo page: **progress** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Determinate and indeterminate are one node | press "Connect": the same indicator stops sweeping and starts filling, and it does **not** jump back to zero first |
//! | Retargetable spring | press "+25%" twice quickly — the fill carries its velocity into the new target instead of restarting |
//! | Correct in both presets | `--preset tailwind`: the track's corners follow the preset, the thickness is a spacing step |
//! | Dark mode | track and fill are tokens |
//! | AccessKit node | announced as a progress indicator with a percentage — and with **no** value while the fraction is unknown, because "stuck at 99%" is the oldest lie in software |
//! | Keyboard + focus ring | deliberately none: a progress indicator takes no input and is not a tab stop |
//! | Reduced motion | the fill keeps moving (it *is* the information) but the indeterminate sweep stops, leaving a static band |
//!
//! ```text
//! cargo run -p silka-gallery -- --page progress
//! ```

use silka_core::app::{component, BuildCtx};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{button, button_variant, progress_bar, progress_circle, ButtonVariant};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Progress";

/// The paragraph under the title.
pub const KETERANGAN: &str = "Two shapes of the same sentence: \"this is going \
    to take a while\". The one that knows its fraction says so; the one that \
    does not know does not pretend to — and both are one node, so switching \
    between them loses none of what is already animating.";

/// The a11y name of the determinate bar.
pub const NAMA_BAR: &str = "Importing invoices";
/// The a11y name of the determinate ring.
pub const NAMA_LINGKARAN: &str = "Uploading attachments";
/// The a11y name of the indicator that flips between the two states.
pub const NAMA_TUKAR: &str = "Connecting to the server";
/// The button that adds a quarter to the progress.
pub const TOMBOL_MAJU: &str = "+25%";
/// The button that puts everything back to zero.
pub const TOMBOL_ULANG: &str = "Retry";
/// The button that turns the unknown fraction into a known one.
pub const TOMBOL_SAMBUNG: &str = "Connect";

/// How much one press of [`TOMBOL_MAJU`] adds.
pub const LANGKAH: f32 = 0.25;

/// The next value after a press: clamped, and wrapping back to zero once full.
///
/// A pure function, and the only rule this page owns — which is why it is
/// tested directly rather than through a window.
pub fn maju(sekarang: f32) -> f32 {
    let berikut = sekarang + LANGKAH;
    if berikut > 1.0 + f32::EPSILON {
        0.0
    } else {
        berikut.clamp(0.0, 1.0)
    }
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    let nilai = use_signal(|| 0.35f32);
    let tersambung = use_signal(|| false);

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [indikator(nilai, tersambung), kendali(&t, nilai, tersambung)],
    )
}

/// The three indicators, in their own component: pressing a button rebuilds
/// this and nothing else (§2.5).
fn indikator(nilai: Signal<f32>, tersambung: Signal<bool>) -> View {
    component("indikator-progress", move |cx| {
        let t = kepala::mulai(cx);
        let v = nilai.get();

        let bar = progress_bar(v).label(NAMA_BAR);
        let ring = progress_circle(v).label(NAMA_LINGKARAN);
        // One node, two states: the fraction only exists once the connection
        // does.
        let tukar = if tersambung.get() {
            progress_circle(v).label(NAMA_TUKAR)
        } else {
            progress_circle(0.0).indeterminate().label(NAMA_TUKAR)
        };

        kepala::spesimen(
            &t,
            "Bar, ring, and the one that does not know yet",
            [
                View::from(bar),
                View::from(
                    row([View::from(ring), View::from(tukar)])
                        .spacing(t.space(6.0))
                        .cross(CrossAlign::Center),
                ),
                kepala::catatan(&t, format!("Value: {}%", (v * 100.0).round() as i32)),
            ],
        )
    })
}

/// The buttons that drive it.
fn kendali(t: &Theme, nilai: Signal<f32>, tersambung: Signal<bool>) -> View {
    column([View::from(
        row([
            View::from(button(TOMBOL_MAJU).on_press(move || nilai.update(|v| *v = maju(*v)))),
            View::from(
                button_variant(TOMBOL_ULANG, ButtonVariant::Secondary)
                    .on_press(move || nilai.set(0.0)),
            ),
            View::from(
                button_variant(TOMBOL_SAMBUNG, ButtonVariant::Secondary)
                    .on_press(move || tersambung.update(|s| *s = !*s)),
            ),
        ])
        .spacing(t.space(3.0))
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .wrap(),
    )])
    .cross(CrossAlign::Center)
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

    /// Pump `n` frames — what the shell does, without a window.
    ///
    /// A fixed count rather than "until nothing moves", because one indicator
    /// on this page is **deliberately** never finished: an indeterminate sweep
    /// loops for as long as it is on screen. The clock is made up because a
    /// test loop runs in microseconds and a real `dt` would leave the springs
    /// standing still (§3.5).
    fn pompa(ui: &mut AppRuntime, n: usize) {
        let mut jam = Instant::now();
        for _ in 0..n {
            ui.animate_at(jam, silka_widgets::advance);
            ui.frame();
            jam += Duration::from_micros(8_333);
        }
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
    fn maju_menaikkan_lalu_kembali_ke_nol_setelah_penuh() {
        assert_eq!(maju(0.0), 0.25);
        assert_eq!(maju(0.75), 1.0);
        // Full plus a step is not 125%: it is the start of the next round.
        assert_eq!(maju(1.0), 0.0);
    }

    #[test]
    fn yang_tahu_pecahannya_menyebutkannya_dan_yang_tidak_tahu_diam() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        for nama in [NAMA_BAR, NAMA_LINGKARAN, NAMA_TUKAR] {
            let e = pohon
                .find_label(nama)
                .unwrap_or_else(|| panic!("{nama} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::ProgressIndicator, "{nama}");
        }
        assert!(
            pohon.find_label(NAMA_BAR).unwrap().node.value.is_some(),
            "pecahan yang diketahui harus diucapkan"
        );
        assert!(
            pohon.find_label(NAMA_TUKAR).unwrap().node.value.is_none(),
            "pecahan yang tidak diketahui tidak boleh dikarang"
        );
    }

    #[test]
    fn menekan_tombol_benar_benar_menggerakkan_nilainya() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();

        let p = kotak(&ui, TOMBOL_MAJU).center();
        klik(&mut ui, p);
        ui.frame();
        assert!(
            silka_widgets::progress::is_animating(ui.tree()),
            "nilai baru harus dikejar dengan spring, bukan dilompati"
        );

        // …and it arrives.
        pompa(&mut ui, 240);
        let nilai = ui
            .access_tree()
            .find_label(NAMA_BAR)
            .and_then(|e| e.node.value.clone())
            .expect("bar menyebutkan nilainya");
        assert_eq!(nilai, "60%", "0.35 + 0.25 = 0.60");
    }

    #[test]
    fn halaman_terbangun_dan_diam_di_kedua_preset() {
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
