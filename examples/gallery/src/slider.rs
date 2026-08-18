//! Demo page: **slider** (`KOMPONEN.md` Tier 2).
//!
//! What the eye can check on this page, one Definition of Done item at a time:
//!
//! - **Drag, click, and keyboard.** Drag the thumb, click in the middle of the
//!   track (the thumb *travels there on a spring*, it does not teleport), then
//!   Tab to the slider and press arrows/Home/End/PageUp — the number changes in
//!   the same place.
//! - **Snapping to steps.** The "Ukuran teks" slider steps by 1pt and "Rentang
//!   harga" by 50; your finger may stop anywhere, the value still lands on a
//!   step.
//! - **Focus ring + hit target.** The focus ring appears around whichever thumb
//!   is active, and the 44pt band around a track only 4pt thick is pressable
//!   even though it is invisible (HIG).
//! - **Two presets + dark mode.** Run with `--preset tailwind` and
//!   `--appearance light|dark`: only tokens change, there is not a single color
//!   number in this file.
//! - **Reduced motion.** With "Reduce motion" on in the OS, the thumb's
//!   magnification goes away while the value's movement stays legible (motion
//!   that explains is never switched off).
//!
//! ```text
//! cargo run -p silka-gallery -- --page slider
//! cargo run -p silka-gallery -- --page slider --preset tailwind --appearance light
//! ```
//!
//! Each row is **its own component**: dragging one slider rebuilds only that
//! row, not the page (§2.5). That is also why the number may be shown as text
//! without making the whole page recompute sixty times a second.

use silka_core::access::AccessRole;
use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, Builder, LayoutProps, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{active_fonts, range_slider, slider, text};

/// The page title.
pub const JUDUL: &str = "Slider";
/// The volume slider's name — also used by the tests to find it in the a11y
/// tree.
pub const VOLUME: &str = "Volume";
/// The text-size slider's name (stepped).
pub const UKURAN: &str = "Ukuran teks";
/// The price-range slider's name (two thumbs).
pub const HARGA: &str = "Rentang harga";
/// The name of the deliberately disabled slider.
pub const MATI: &str = "Sedang dikunci";

/// The control column's maximum width, in spacing steps (§2.6).
const LEBAR_LANGKAH: f32 = 120.0;

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    // The signals are created in the root scope but **read in each row's
    // component** — that is what makes a drag rebuild only one row.
    let volume = use_signal(|| 40.0f32);
    let ukuran = use_signal(|| 15.0f32);
    let harga_min = use_signal(|| 200.0f32);
    let harga_max = use_signal(|| 800.0f32);

    let isi = column([
        View::from(
            text(JUDUL)
                .size(t.typography.body_size * 2.0)
                .weight(FontWeight::SEMIBOLD)
                .tracking(-0.02)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                "Seret, klik di track, atau Tab lalu tekan panah. Nilainya \
                 mendarat di undakan; thumb-nya menyusul lewat spring.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label),
        ),
        baris_volume(volume),
        baris_ukuran(ukuran),
        baris_harga(harga_min, harga_max),
        baris_mati(&t),
    ])
    .spacing(t.space(7.0))
    .cross(CrossAlign::Stretch)
    .padding(Insets::all(t.space(8.0)));

    // The control column is never as wide as the window: an over-wide form
    // makes a slider impossible to use precisely.
    constrained(
        BoxConstraints::new(0.0, t.space(LEBAR_LANGKAH), 0.0, f32::INFINITY),
        isi,
    )
    .into()
}

/// One row's heading: name on the left, value on the right.
///
/// Both texts take the [`AccessRole::Container`] role so a screen reader does
/// not announce "Volume, Volume, 40": the name is already attached to the
/// slider, and the value rides on that same node (§3.8).
fn kepala(t: &Theme, nama: &str, nilai: String) -> Builder<LayoutProps> {
    row([
        View::from(
            text(nama)
                .size(t.typography.body_size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.label)
                .single_line()
                .role(AccessRole::Container),
        ),
        View::from(
            text(nilai)
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                .role(AccessRole::Container),
        ),
    ])
    .main(MainAlign::SpaceBetween)
    .cross(CrossAlign::Center)
}

/// A continuous 0–100 slider.
fn baris_volume(volume: Signal<f32>) -> View {
    component("volume", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let v = volume.get();
        column([
            View::from(kepala(&t, VOLUME, format!("{v:.0}%"))),
            View::from(
                slider(v)
                    .range(0.0..=100.0)
                    .label(VOLUME)
                    .on_change(move |x| volume.set(x)),
            ),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .into()
    })
}

/// A stepped slider — the snapping to steps `KOMPONEN.md` asks for.
fn baris_ukuran(ukuran: Signal<f32>) -> View {
    component("ukuran", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let v = ukuran.get();
        column([
            View::from(kepala(&t, UKURAN, format!("{v:.0} pt"))),
            View::from(
                slider(v)
                    .range(9.0..=32.0)
                    .step(1.0)
                    .label(UKURAN)
                    .on_change(move |x| ukuran.set(x)),
            ),
            // A live example: the text really is the size of the slider's
            // value.
            View::from(
                text("Ukuran teks mengikuti nilai di atas.")
                    .size(v)
                    .color(t.color.label)
                    .single_line()
                    .role(AccessRole::Container),
            ),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .into()
    })
}

/// The range variant: two thumbs that must not cross each other.
fn baris_harga(min: Signal<f32>, max: Signal<f32>) -> View {
    component("harga", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let (a, b) = (min.get(), max.get());
        column([
            View::from(kepala(&t, HARGA, format!("{a:.0} – {b:.0}"))),
            View::from(
                range_slider(a, b)
                    .range(0.0..=1000.0)
                    .step(50.0)
                    .label(HARGA)
                    .on_range_change(move |x, y| {
                        min.set(x);
                        max.set(y);
                    }),
            ),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .into()
    })
}

/// The disabled slider: a screen reader still announces it as dimmed.
fn baris_mati(t: &Theme) -> View {
    column([
        View::from(kepala(t, MATI, "60".to_string())),
        View::from(slider(60.0).range(0.0..=100.0).label(MATI).disabled(true)),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Stretch)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessActions;
    use silka_core::animation::Motion;
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_core::scheduler::Dirty;
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::slider::sliders;
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(720.0, 640.0);

    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, move |cx| halaman(cx)).sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// A slider's a11y node, looked up by name.
    fn slider_a11y(ui: &AppRuntime, nama: &str) -> (Rect, Option<String>, bool) {
        let pohon = ui.access_tree();
        let e = pohon
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Slider && e.node.label.as_deref() == Some(nama))
            .unwrap_or_else(|| panic!("slider {nama:?} hilang:\n{}", pohon.dump()));
        (e.bounds, e.node.value.clone(), e.node.disabled)
    }

    fn nilai(ui: &AppRuntime, nama: &str) -> f32 {
        slider_a11y(ui, nama)
            .1
            .expect("slider selalu membawa nilainya")
            .parse()
            .expect("nilai slider satu thumb selalu satu angka")
    }

    /// One full click (move, press, release) at point `p`.
    fn klik(ui: &mut AppRuntime, p: Point) {
        for (fase, ms) in [
            (PointerPhase::Move, 0),
            (PointerPhase::Down, 8),
            (PointerPhase::Up, 40),
        ] {
            let mut e = PointerEvent::new(fase, p, Duration::from_millis(ms));
            if matches!(fase, PointerPhase::Down | PointerPhase::Up) {
                e = e.button(PointerButton::Primary);
            }
            ui.dispatch(&Event::Pointer(e));
        }
    }

    fn tombol(ui: &mut AppRuntime, key: NamedKey) {
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
    }

    /// Pump frames until every spring stops (at most `batas` frames).
    fn sampai_diam(ui: &mut AppRuntime, batas: usize) -> usize {
        let mut now = Instant::now();
        let mut n = 0;
        while n < batas {
            let dirty = ui.animate_at(now, silka_widgets::advance);
            ui.frame();
            n += 1;
            if !dirty.contains(Dirty::ANIMATION) {
                break;
            }
            now += Duration::from_micros(8_333);
        }
        n
    }

    #[test]
    fn keempat_slider_ada_di_pohon_a11y_dengan_hit_target_hig() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        for nama in [VOLUME, UKURAN, HARGA, MATI] {
            let (bounds, nilai, _) = slider_a11y(&ui, nama);
            assert!(
                bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {nama} cuma {:?}",
                bounds.size
            );
            assert!(nilai.is_some(), "{nama} tidak membawa nilainya");
            assert!(bounds.size.width > 100.0, "{nama} nyaris tak bisa dipakai");
        }

        assert_eq!(nilai(&ui, VOLUME), 40.0);
        assert_eq!(nilai(&ui, UKURAN), 15.0);
        assert_eq!(slider_a11y(&ui, HARGA).1.as_deref(), Some("200 – 800"));
        assert!(slider_a11y(&ui, MATI).2, "slider terkunci harus dimmed");
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_di_track_menggeser_nilai_dan_hanya_membangun_ulang_satu_baris() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();

        let kotak = slider_a11y(&ui, VOLUME).0;
        // Three quarters along the track: the value should be close to 75.
        klik(
            &mut ui,
            Point::new(kotak.min_x() + kotak.size.width * 0.75, kotak.center().y),
        );
        let laporan = ui.frame();
        let v = nilai(&ui, VOLUME);
        assert!((v - 75.0).abs() < 4.0, "klik di 75% → {v}");
        assert_eq!(
            laporan.rebuilt, 1,
            "hanya baris volume yang membaca signalnya"
        );
        // The other rows do not move along with it.
        assert_eq!(nilai(&ui, UKURAN), 15.0);
    }

    #[test]
    fn keyboard_menggeser_slider_berundak_tanpa_mouse() {
        let mut ui = ui(Theme::tailwind(Appearance::Dark));
        ui.frame();

        // Tab until the "Ukuran teks" slider holds focus.
        for _ in 0..2 {
            tombol(&mut ui, NamedKey::Tab);
        }
        ui.frame();
        tombol(&mut ui, NamedKey::ArrowRight);
        ui.frame();
        assert_eq!(nilai(&ui, UKURAN), 16.0, "panah kanan = satu undakan");

        tombol(&mut ui, NamedKey::End);
        ui.frame();
        assert_eq!(nilai(&ui, UKURAN), 32.0);
        tombol(&mut ui, NamedKey::Home);
        ui.frame();
        assert_eq!(nilai(&ui, UKURAN), 9.0);
        // The other sliders are not dragged along by the focus.
        assert_eq!(nilai(&ui, VOLUME), 40.0);
    }

    #[test]
    fn slider_terkunci_tidak_bisa_digeser_maupun_difokuskan() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        let kotak = slider_a11y(&ui, MATI).0;
        klik(&mut ui, Point::new(kotak.max_x() - 8.0, kotak.center().y));
        ui.frame();
        assert_eq!(slider_a11y(&ui, MATI).1.as_deref(), Some("60"));

        let pohon = ui.access_tree();
        let e = pohon
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Slider && e.node.label.as_deref() == Some(MATI))
            .unwrap();
        assert!(!e.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn thumb_menyusul_nilainya_lewat_spring_lalu_gpu_kembali_tidur() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        // The first frame primes the animation pump.
        ui.animate_at(Instant::now(), silka_widgets::advance);
        ui.frame();

        for _ in 0..2 {
            tombol(&mut ui, NamedKey::Tab);
        }
        ui.frame();
        tombol(&mut ui, NamedKey::End);
        ui.frame();

        // The value is already at the end, but the thumb is still on its way.
        assert_eq!(nilai(&ui, UKURAN), 32.0);
        assert!(silka_widgets::is_animating(ui.tree()));

        let frame = sampai_diam(&mut ui, 600);
        assert!(frame > 1, "gerakan selesai seketika — itu lompatan");
        assert!(frame < 600, "spring tidak pernah settle");
        assert!(!silka_widgets::is_animating(ui.tree()));
        assert!(ui.is_idle(), "GPU tidak kembali tidur setelah spring diam");
    }

    #[test]
    fn reduced_motion_tidak_menghentikan_perpindahan_nilai() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        for _ in 0..2 {
            tombol(&mut ui, NamedKey::Tab);
        }
        ui.frame();
        tombol(&mut ui, NamedKey::End);
        ui.frame();

        // The OS "reduce motion" preference enters through the runtime, not
        // through the widget: one place, applying to the whole tree.
        ui.set_motion(Motion::Reduced);
        let n = sampai_diam(&mut ui, 600);
        assert!(n < 600, "gerakan yang menjelaskan ikut dimatikan");
        assert_eq!(nilai(&ui, UKURAN), 32.0);
        assert!(!silka_widgets::is_animating(ui.tree()));
    }

    #[test]
    fn warna_selalu_datang_dari_token_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                let latar: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.background),
                        _ => None,
                    })
                    .collect();
                for w in &latar {
                    assert!(
                        *w == t.color.surface_sunken
                            || *w == t.color.accent
                            || *w == t.color.accent_muted
                            || *w == t.color.surface_elevated
                            || w.a == 0.0,
                        "warna lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
                // Four tracks, four fills, five thumbs (two on the range
                // slider).
                assert!(
                    latar
                        .iter()
                        .filter(|w| **w == t.color.surface_sunken)
                        .count()
                        == 4,
                    "harus ada empat track"
                );
            }
        }
    }

    #[test]
    fn setiap_slider_punya_node_render_sendiri() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        assert_eq!(sliders(ui.tree()).len(), 4);
    }
}
