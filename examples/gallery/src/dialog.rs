//! Demo page: **dialogs & alerts** (`KOMPONEN.md` Tier 4).
//!
//! What the eye checks on this page, one Definition of Done item at a time:
//!
//! - **A dimmed backdrop + centered panel** coming from the `scrim` and
//!   `surface_elevated` tokens — compare across both presets
//!   (`--preset tailwind`) and both appearances: the corners are squircles in
//!   Cupertino, arcs in Tailwind.
//! - **A retargetable spring transition**: press the opening button and then
//!   immediately press Esc — the dialog reverses carrying its velocity, it does
//!   not first jump to zero.
//! - **Per-OS button convention**: the first dialog uses
//!   [`ButtonOrder::Platform`] (on macOS "Batal" on the left, "Simpan" on the
//!   right), while the two dialogs below it force both orderings so they can be
//!   seen side by side without switching OS.
//! - **Keyboard**: Tab enters the dialog's focus trap and never escapes to the
//!   content behind it, Space/Enter activates the focused button, Return runs
//!   the default button from anywhere **inside** the dialog, and Esc runs the
//!   cancel action.
//! - **An alert does not vanish because the cursor slipped**: clicking outside
//!   the panel closes an ordinary dialog, but not an alert (`NSAlert`).
//! - **Reduced motion**: the transition is marked `Essential`, so under that
//!   setting the panel still moves (the motion explains where the dialog came
//!   from) but the bounce is dropped. Not yet observable from this page — the
//!   shell does not read the OS setting yet (INTEGRASI-NATIVE §6) — so what
//!   guards it is the `silka_widgets::dialog` test that runs the same
//!   transition under `Motion::Reduced`.
//!
//! ```text
//! cargo run -p silka-gallery -- --page dialog
//! cargo run -p silka-gallery -- --page dialog --preset tailwind --appearance light
//! ```
//!
//! One limitation is named honestly here because it shows up immediately:
//! **focus does not move automatically** to a freshly opened panel (a gap
//! already recorded in `silka_widgets::overlay`), so after a dialog appears via
//! a click, press Tab once to enter its focus trap. The safety net for the
//! "nothing is focused yet" case already exists as shell functions —
//! `overlay::dismiss_topmost` and `dialog::activate_default` — but wiring them
//! up is the application's input cycle's job, and `run_app_with` has no hook
//! for that yet.

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    alert, button, button_variant, dialog, overlay_layer, text, ButtonOrder, ButtonVariant, Fonts,
};

/// The page title.
pub const JUDUL: &str = "Dialog";

/// The button that opens the ordinary dialog.
pub const BUKA_SIMPAN: &str = "Simpan perubahan…";
/// The button that opens the destructive alert.
pub const BUKA_HAPUS: &str = "Hapus berkas…";
/// The button that opens the dialog with Windows-style button order.
pub const BUKA_WINDOWS: &str = "Susunan Windows…";

/// The ordinary dialog's title.
pub const JUDUL_SIMPAN: &str = "Simpan perubahan?";
/// The destructive alert's title.
pub const JUDUL_HAPUS: &str = "Hapus 3 berkas?";
/// The title of the Windows-button-order example dialog.
pub const JUDUL_WINDOWS: &str = "Susunan tombol Windows";

/// The answer before the user presses anything.
pub const BELUM_DIJAWAB: &str = "belum ada";

/// Which dialog is currently open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Buka {
    /// None.
    #[default]
    Tidak,
    /// The "Simpan perubahan?" dialog.
    Simpan,
    /// The "Hapus 3 berkas?" alert.
    Hapus,
    /// The Windows-button-order example dialog.
    Windows,
}

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution; the logical sizes
    // below do not change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let buka = use_signal(|| Buka::Tidak);
    let jawaban = use_signal(|| String::from(BELUM_DIJAWAB));

    // `jawaban` is written from inside the dialog and read on the page: proof
    // that the button clicked really does run its action, not merely close the
    // panel.
    let jawab = move |apa: &'static str| {
        move || {
            jawaban.set(apa.to_string());
            buka.set(Buka::Tidak);
        }
    };
    let tutup = move || buka.set(Buka::Tidak);

    overlay_layer(konten(fonts, &t, buka, jawaban))
        .overlay(
            dialog(fonts, &t, JUDUL_SIMPAN)
                .message(
                    "Dokumen ini punya perubahan yang belum disimpan. \
                     Menutupnya sekarang akan membuang perubahan itu.",
                )
                .open(buka.get() == Buka::Simpan)
                .action(silka_widgets::action("Jangan Simpan").on_press(jawab("Jangan Simpan")))
                .cancel("Batal", jawab("Batal"))
                .confirm("Simpan", jawab("Simpan")),
        )
        .overlay(
            // A destructive alert: clicking outside does not close it, and
            // Return never runs "Hapus" (HIG).
            alert(fonts, &t, JUDUL_HAPUS)
                .message("Berkas yang dihapus tidak bisa dikembalikan.")
                .open(buka.get() == Buka::Hapus)
                .cancel("Batal", jawab("Batal"))
                .destructive("Hapus", jawab("Hapus")),
        )
        .overlay(
            dialog(fonts, &t, JUDUL_WINDOWS)
                .message(
                    "Susunan yang sama dipaksa ke konvensi Windows: tombol \
                     default di kiri, batal di kanannya.",
                )
                .open(buka.get() == Buka::Windows)
                .order(ButtonOrder::ConfirmFirst)
                .cancel("Batal", tutup)
                .confirm("Ok", tutup),
        )
        .into()
}

/// The page content behind the dialog — inert while a modal is open.
fn konten(fonts: &Fonts, t: &Theme, buka: Signal<Buka>, jawaban: Signal<String>) -> View {
    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Modal dengan backdrop dim di atas sistem overlay yang sama \
                 dengan popover dan toast. Urutan tombolnya mengikuti konvensi \
                 OS; Esc membatalkan, Return menjalankan tombol default.",
            )
            .size(t.typography.body.size)
            .line_height(t.typography.body.line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        View::from(
            text(fonts, format!("Jawaban terakhir: {}", jawaban.get()))
                .size(t.typography.callout.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        View::from(
            row([
                View::from(button(fonts, t, BUKA_SIMPAN).on_press(move || buka.set(Buka::Simpan))),
                View::from(
                    button_variant(fonts, t, BUKA_HAPUS, ButtonVariant::Destructive)
                        .on_press(move || buka.set(Buka::Hapus)),
                ),
                View::from(
                    button_variant(fonts, t, BUKA_WINDOWS, ButtonVariant::Secondary)
                        .on_press(move || buka.set(Buka::Windows)),
                ),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center)
            .wrap(),
        ),
    ])
    .spacing(t.space(6.0))
    // The alignment belongs to the layout engine, not to arithmetic on this
    // page (§3.4).
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Scene, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 640.0);
    /// One 60 Hz frame — a fake clock, because tests must not wait on real
    /// time to let springs move (§9.5).
    const FRAME: Duration = Duration::from_millis(16);

    /// This page inside **exactly the same** lifecycle as `run_app_with`:
    /// animate → frame, with a clock the test controls.
    struct Uji {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Uji {
        fn baru(theme: Theme, fonts: &Fonts) -> Self {
            let untuk_view = fonts.clone();
            let ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
                .sized(VIEWPORT.width, VIEWPORT.height);
            Self {
                ui,
                jam: Instant::now(),
            }
        }

        /// One frame, springs stepped included — the same order as the shell.
        fn frame(&mut self) {
            self.jam += FRAME;
            self.ui.animate_at(self.jam, silka_widgets::advance);
            self.ui.frame();
        }

        /// Pump frames until nothing is moving any more.
        fn diam(&mut self) -> u32 {
            let mut n = 0;
            while !self.ui.is_idle() {
                self.frame();
                n += 1;
                assert!(n < 600, "halaman tidak pernah diam");
            }
            n
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
        }

        fn tombol(&mut self, label: &str) {
            let p = self.kotak(label).center();
            self.klik(p);
        }

        fn key(&mut self, named: NamedKey) {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(named),
                Duration::ZERO,
            )));
        }

        fn jawaban(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with("Jawaban terakhir: "))
                .unwrap_or_else(|| panic!("baris jawaban hilang:\n{}", pohon.dump()))
        }

        fn scene(&self) -> &Scene {
            self.ui.scene()
        }
    }

    /// A deterministic text engine: no system fonts (§9.5).
    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    #[test]
    fn halaman_dimulai_tanpa_dialog_dan_benar_benar_diam() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        uji.diam();

        assert!(!uji.ada(JUDUL_SIMPAN));
        for label in [BUKA_SIMPAN, BUKA_HAPUS, BUKA_WINDOWS] {
            assert!(uji.ada(label), "{label} hilang");
        }
        assert!(uji.ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_membuka_dialog_yang_beranimasi_masuk_lalu_diam() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light), &f);
        uji.diam();

        uji.tombol(BUKA_SIMPAN);
        assert!(!uji.ui.is_idle(), "klik harus menjadwalkan frame");
        uji.frame();

        // The dialog is in the a11y tree from the very first frame…
        let a11y = uji.ui.access_tree();
        let d = a11y
            .find_label(JUDUL_SIMPAN)
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(d.node.role, AccessRole::Dialog);
        // …and the content behind it is already inert.
        assert!(
            a11y.find_label(BUKA_SIMPAN).is_none(),
            "konten di belakang modal masih dibacakan:\n{}",
            a11y.dump()
        );

        // …but it is still moving: the transition is a spring, not a jump.
        // This is a regression that actually happened once — an animation
        // **started by the view-diff** (the `open` prop changed) must still
        // schedule the next frame.
        assert!(
            !uji.ui.is_idle(),
            "panel yang baru muncul harus meminta frame berikutnya"
        );
        let frame = uji.diam();
        assert!(frame > 1, "transisi harus memakan lebih dari satu frame");

        let panel = uji.kotak(JUDUL_SIMPAN);
        assert!(
            (panel.center().x - VIEWPORT.width / 2.0).abs() < 1.0,
            "{panel:?}"
        );
    }

    #[test]
    fn tombol_dialog_menjawab_lalu_menutup() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        uji.diam();
        assert!(uji.jawaban().ends_with(BELUM_DIJAWAB));

        uji.tombol(BUKA_SIMPAN);
        uji.diam();
        uji.tombol("Simpan");
        uji.diam();

        assert!(uji.jawaban().ends_with("Simpan"));
        assert!(
            !uji.ada(JUDUL_SIMPAN),
            "setelah transisi keluar habis, dialog benar-benar tidak ada"
        );
        // The content behind it comes back to life.
        assert!(uji.ada(BUKA_SIMPAN));
    }

    #[test]
    fn esc_membatalkan_setelah_fokus_masuk_ke_dialog() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark), &f);
        uji.diam();
        uji.tombol(BUKA_SIMPAN);
        uji.diam();

        // Tab enters the dialog's focus trap; Esc then bubbles through the
        // overlay entry and runs the cancel action.
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Escape);
        uji.diam();

        assert!(uji.jawaban().ends_with("Batal"));
        assert!(!uji.ada(JUDUL_SIMPAN));
    }

    #[test]
    fn keyboard_mengaktifkan_tombol_dialog_tanpa_mouse() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light), &f);
        uji.diam();
        uji.tombol(BUKA_WINDOWS);
        uji.diam();

        // The first Tab lands on the dialog itself (where a modal lands), the
        // second on the first button — which under the Windows order is the
        // default one — and then Space activates it.
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Space);
        uji.diam();
        assert!(!uji.ada(JUDUL_WINDOWS));
    }

    #[test]
    fn klik_di_luar_menutup_dialog_tapi_tidak_menutup_alert() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        let pojok = Point::new(6.0, 6.0);

        uji.diam();
        uji.tombol(BUKA_SIMPAN);
        uji.diam();
        uji.klik(pojok);
        uji.diam();
        assert!(!uji.ada(JUDUL_SIMPAN), "dialog biasa: klik luar = batal");
        assert!(uji.jawaban().ends_with("Batal"));

        uji.tombol(BUKA_HAPUS);
        uji.diam();
        uji.klik(pojok);
        uji.diam();
        assert!(
            uji.ada(JUDUL_HAPUS),
            "alert tidak boleh hilang karena kursor tergelincir"
        );
    }

    #[test]
    fn warna_halaman_dan_panel_selalu_datang_dari_token() {
        let f = fonts();
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut uji = Uji::baru(t, &f);
                uji.diam();
                assert_eq!(uji.scene().clear_color(), t.color.background);

                uji.tombol(BUKA_SIMPAN);
                uji.diam();

                let kotak: Vec<_> = uji
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(
                    kotak
                        .iter()
                        .any(|q| q.background == t.color.scrim && q.rect.size == VIEWPORT),
                    "{preset:?}/{appearance:?}: backdrop bukan token scrim"
                );
                // Found by width, not by color alone: in the Cupertino preset
                // `surface_elevated` equals `surface`, so secondary buttons
                // share the panel's background.
                let lebar = t.space(silka_widgets::DIALOG_WIDTH_STEPS);
                let panel = kotak
                    .iter()
                    .find(|q| {
                        q.background == t.color.surface_elevated
                            && (q.rect.size.width - lebar).abs() < 0.5
                    })
                    .unwrap_or_else(|| panic!("{preset:?}/{appearance:?}: panel tidak tergambar"));
                assert_eq!(panel.corners.style, t.radius.style);
                assert_eq!(panel.corners.radii.max(), t.radius.xl);
            }
        }
    }

    #[test]
    fn setiap_tombol_dialog_bisa_diklik_dan_memenuhi_hit_target() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        uji.diam();
        uji.tombol(BUKA_SIMPAN);
        uji.diam();

        let pohon = uji.ui.access_tree();
        for label in ["Simpan", "Batal", "Jangan Simpan"] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::Button);
            assert!(e.node.actions.contains(AccessActions::CLICK));
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
    }
}
