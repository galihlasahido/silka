//! Demo page: **scroll_view** (`KOMPONEN.md` Tier 1).
//!
//! `KOMPONEN.md` calls scrolling "the earliest native-feel differentiator a user
//! notices" — and that feel is the one thing no unit test can prove. Hence this
//! page: every row of the table below is something that must **feel right in
//! the hand**, not merely be green in CI.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | macOS-style rubber banding | Scroll past the top/bottom edge: the content stretches with growing resistance, then springs back |
//! | The OS's own momentum | Fling with two fingers on the trackpad: the inertial tail is macOS's, not multiplied by us |
//! | Fling → spring handoff | Fling until it hits the edge: the bounce **continues the velocity** of the fling (§3.5) |
//! | Smooth mouse wheel | One wheel click glides on a spring, it does not jump |
//! | Auto-hiding overlay scrollbar | The bar appears while scrolling, then fades on its own once things settle |
//! | Scrollbar widens on hover | Bring the cursor near the right edge: the bar widens on a spring, and its track appears with it |
//! | Dragging the thumb | Drag the bar directly: the content follows instantly, with no animation |
//! | Full keyboard support + focus ring | Tab to the list, then ↑ ↓ PageUp/PageDown Home/End; the focus ring is visible |
//! | Scroll-to | The "Ke atas"/"Tengah"/"Ke bawah" buttons |
//! | Both presets & dark mode | `--preset tailwind`, `--appearance dark` |
//! | AccessKit nodes | VoiceOver says "scroll view" along with its position as a percentage |
//! | Reduced motion | Turn on "Reduce motion" in the OS: scrolling **ends up in the same place**, only the glide goes away |
//!
//! What is **absent** from this file: hand-assembled `Scene`s, layout
//! arithmetic, and color numbers. Everything is a token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{button_in, button_variant_in, scroll_view_in, text_in, ButtonVariant, Fonts};

/// The page title.
pub const JUDUL: &str = "Scroll view";
/// The list's name for screen readers — and the anchor the tests look for.
pub const NAMA_DAFTAR: &str = "Daftar transaksi";
/// How many rows are in the list.
pub const BARIS: usize = 40;

/// The scroll-to-top button.
pub const TOMBOL_ATAS: &str = "Ke atas";
/// The scroll-to-middle button.
pub const TOMBOL_TENGAH: &str = "Tengah";
/// The scroll-to-bottom button.
pub const TOMBOL_BAWAH: &str = "Ke bawah";

/// The list viewport's height, in **spacing-scale steps** (§2.6) — not a free
/// number.
const TINGGI_LANGKAH: f32 = 90.0;
/// The list's maximum width, in spacing-scale steps.
const LEBAR_LANGKAH: f32 = 150.0;

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // The position **owned by the application**: only the scroll-to buttons
    // write it. The mouse wheel and trackpad never touch it — the day-to-day
    // scroll position belongs to the node, and that is what prevents the
    // "controlled component" bug that throws the user back to the top every
    // time a signal changes.
    let tujuan = use_signal(|| 0.0f32);

    column([
        View::from(
            text_in(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text_in(
                fonts,
                "Gulir melewati ujungnya: isinya melar makin berat lalu memantul \
                 pulang — rubber band ala macOS. Momentum trackpad datang dari OS \
                 apa adanya; yang kita kerjakan hanya pantulannya, dan pantulan \
                 itu melanjutkan kecepatan lemparan.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        daftar(fonts, &t, tujuan),
        kendali(fonts, &t, tujuan),
        View::from(
            text_in(
                fonts,
                "Keyboard: Tab ke daftar, lalu ↑ ↓ · Page Up/Down · Home/End · Spasi.",
            )
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line(),
        ),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The list viewport: **the only place `tujuan` is read**, so pressing a
/// scroll-to button rebuilds just this section (§2.5).
fn daftar(fonts: &Fonts, t: &Theme, tujuan: Signal<f32>) -> View {
    let fonts = fonts.clone();
    let theme = *t;
    component("daftar", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let isi = column((0..BARIS).map(|i| baris(&fonts, &t, i)));

        // The scroll axis **must** be bounded (the same rule as Flutter's):
        // the bound lives here, not inside the container.
        constrained(
            BoxConstraints::new(
                0.0,
                t.space(LEBAR_LANGKAH),
                t.space(TINGGI_LANGKAH),
                t.space(TINGGI_LANGKAH),
            ),
            scroll_view_in(&t, isi)
                .label(NAMA_DAFTAR)
                .scroll(tujuan.get())
                .background(t.color.surface_sunken)
                .corners(t.corners(t.radius.lg))
                .border(t.space(0.25), t.color.separator),
        )
        .into()
    })
}

/// One list row — striped so that scrolling is visibly moving.
fn baris(fonts: &Fonts, t: &Theme, i: usize) -> View {
    let genap = i % 2 == 0;
    let latar = if genap {
        t.color.surface
    } else {
        t.color.surface_hover
    };
    let kiri = text_in(fonts, format!("Transaksi #{:02}", i + 1))
        .size(t.typography.body_size)
        .color(t.color.label)
        .single_line();
    let kanan = text_in(fonts, format!("Rp {}.000", (i + 1) * 125))
        .size(t.typography.body_size)
        .weight(FontWeight::MEDIUM)
        .color(t.color.secondary_label)
        .single_line();

    row([View::from(kiri), View::from(kanan)])
        .key(i as i64)
        .main(MainAlign::SpaceBetween)
        .cross(CrossAlign::Center)
        .padding(Insets::symmetric(t.space(4.0), t.space(3.0)))
        .background(latar)
        .into()
}

/// The three scroll-to buttons.
///
/// The value written is an **absolute position**; the container clamps it to
/// the maximum scroll itself, so this page need not know how tall the content
/// turns out to be once the text is laid out.
fn kendali(fonts: &Fonts, t: &Theme, tujuan: Signal<f32>) -> View {
    row([
        View::from(
            button_variant_in(fonts, t, TOMBOL_ATAS, ButtonVariant::Secondary)
                .on_press(move || tujuan.set(0.0)),
        ),
        View::from(
            button_variant_in(fonts, t, TOMBOL_TENGAH, ButtonVariant::Secondary)
                // Half the content height; the container clamps the rest.
                .on_press(move || tujuan.set(BARIS as f32 * 24.0)),
        ),
        View::from(button_in(fonts, t, TOMBOL_BAWAH).on_press(move || tujuan.set(f32::MAX))),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerId,
        PointerPhase, ScrollDelta, ScrollEvent, ScrollPhase,
    };
    use silka_core::scheduler::Dirty;
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::scroll_view::{nodes, ScrollView};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 700.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// One complete frame, animation tick included — the same order as the
    /// shell (`silka_platform::run_app_with`).
    fn frame(ui: &mut AppRuntime, waktu: Instant) -> Dirty {
        let dirty = ui.animate_at(waktu, silka_widgets::advance);
        ui.frame();
        dirty
    }

    /// Pump frames until the app is genuinely **idle** — not merely until the
    /// springs stop.
    ///
    /// The difference matters: after scrolling settles there is still the
    /// scrollbar's auto-hide countdown asking for frames, and the "render only
    /// when dirty" promise (§3.5) is only proven once *that* ends too.
    fn selesaikan(ui: &mut AppRuntime) {
        let mut waktu = Instant::now();
        for _ in 0..600 {
            waktu += Duration::from_millis(16);
            frame(ui, waktu);
            if ui.is_idle() {
                return;
            }
        }
        panic!("halaman tidak pernah berhenti beranimasi");
    }

    fn gulir_node(ui: &AppRuntime) -> &ScrollView {
        let id = *nodes(ui.tree())
            .first()
            .expect("ada scroll_view di halaman");
        ui.tree().node_ref::<ScrollView>(id).expect("node gulir")
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn roda(ui: &mut AppRuntime, titik: Point, dy: f32) {
        ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: titik,
            delta: ScrollDelta::Lines { x: 0.0, y: dy },
            phase: ScrollPhase::Wheel,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
        }));
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
    fn halaman_punya_daftar_yang_bisa_digulir_dan_dibacakan() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let e = pohon
            .find_label(NAMA_DAFTAR)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(e.node.role, AccessRole::ScrollView);
        assert!(e.node.actions.contains(AccessActions::SCROLL));
        assert!(e.node.actions.contains(AccessActions::FOCUS));
        assert_eq!(e.node.value.as_deref(), Some("0%"));

        let gulir = gulir_node(&ui);
        assert!(
            gulir.content() > gulir.extent(),
            "isi harus lebih tinggi dari jendelanya: {gulir:?}"
        );
        assert!(gulir.thumb().is_some(), "scrollbar punya thumb");
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn roda_mouse_menggulir_daftar_lewat_spring() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        let tengah = kotak(&ui, NAMA_DAFTAR).center();

        roda(&mut ui, tengah, -3.0);
        assert!(!ui.is_idle(), "guliran menjadwalkan frame");

        // Two frames: the first has a `dt` of zero (the animation clock has
        // only just started, see `AnimationDriver::begin_frame`), the second
        // actually advances. Moving but not yet arrived — that is the
        // difference between a spring and a jump.
        let waktu = Instant::now();
        frame(&mut ui, waktu);
        frame(&mut ui, waktu + Duration::from_millis(16));
        let separuh = gulir_node(&ui).offset();
        assert!(separuh > 0.0, "harus mulai bergerak");
        assert!(separuh < gulir_node(&ui).target());

        selesaikan(&mut ui);
        assert!(gulir_node(&ui).offset() > 0.0);
        assert_eq!(gulir_node(&ui).offset(), gulir_node(&ui).target());

        // What a screen reader announces changes along with the pixels.
        let persen = ui
            .access_tree()
            .find_label(NAMA_DAFTAR)
            .and_then(|e| e.node.value.clone())
            .expect("posisi dibacakan");
        assert_ne!(persen, "0%");
    }

    #[test]
    fn tombol_scroll_to_membawa_daftar_ke_ujungnya() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Dark), &f);
        ui.frame();

        let titik = kotak(&ui, TOMBOL_BAWAH).center();
        klik(&mut ui, titik);
        selesaikan(&mut ui);
        let bawah = gulir_node(&ui);
        assert_eq!(
            bawah.offset(),
            bawah.max_scroll(),
            "harus mendarat tepat di dasar"
        );
        assert!(bawah.max_scroll() > 0.0);

        let titik = kotak(&ui, TOMBOL_ATAS).center();
        klik(&mut ui, titik);
        selesaikan(&mut ui);
        assert_eq!(gulir_node(&ui).offset(), 0.0);
    }

    #[test]
    fn keyboard_menggulir_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        // Tab until the list holds focus, then End takes it to the bottom.
        let id = *nodes(ui.tree()).first().expect("ada scroll_view");
        for _ in 0..8 {
            if ui.router().focus().focused() == Some(id) {
                break;
            }
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )));
        }
        assert_eq!(
            ui.router().focus().focused(),
            Some(id),
            "daftar harus bisa dijangkau Tab"
        );

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::End),
            Duration::from_millis(20),
        )));
        selesaikan(&mut ui);
        let g = gulir_node(&ui);
        assert_eq!(g.offset(), g.max_scroll());
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_appearance() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                // The list's background and corner shape come from tokens, and
                // that shape differs between presets (squircle vs arc).
                let latar = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        silka_paint::Command::Quad(q) if q.background == t.color.surface_sunken => {
                            Some(q.clone())
                        }
                        _ => None,
                    })
                    .next()
                    .unwrap_or_else(|| panic!("{preset:?}: latar daftar tidak digambar"));
                assert_eq!(latar.corners.style, t.radius.style);
                assert!(latar.border_width > 0.0);
                assert_eq!(latar.border_color, t.color.separator);
            }
        }
    }

    #[test]
    fn menggulir_dengan_trackpad_memantul_lalu_diam() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let tengah = kotak(&ui, NAMA_DAFTAR).center();

        // A finger drags downward past the top edge: the content stretches.
        let mut router_time = 0u64;
        for phase in [
            ScrollPhase::Began,
            ScrollPhase::Changed,
            ScrollPhase::Changed,
        ] {
            router_time += 16;
            ui.dispatch(&Event::Scroll(ScrollEvent {
                id: PointerId::MOUSE,
                position: tengah,
                delta: ScrollDelta::Points { x: 0.0, y: 90.0 },
                phase,
                modifiers: Modifiers::NONE,
                time: Duration::from_millis(router_time),
            }));
        }
        assert!(
            gulir_node(&ui).is_overscrolled(),
            "harus melar: {:?}",
            gulir_node(&ui)
        );

        // The finger lifts → the spring bounces, then genuinely stops.
        ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: tengah,
            delta: ScrollDelta::Points { x: 0.0, y: 0.0 },
            phase: ScrollPhase::Ended,
            modifiers: Modifiers::NONE,
            time: Duration::from_millis(router_time + 16),
        }));
        selesaikan(&mut ui);
        assert_eq!(gulir_node(&ui).offset(), 0.0);
        assert!(!gulir_node(&ui).is_overscrolled());
        assert!(ui.is_idle(), "setelah semuanya diam, GPU boleh tidur");
    }

    #[test]
    fn scrollbar_muncul_saat_digulir_lalu_memudar() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert_eq!(gulir_node(&ui).bar_opacity(), 0.0);

        let tengah = kotak(&ui, NAMA_DAFTAR).center();
        roda(&mut ui, tengah, -2.0);
        let mut waktu = Instant::now();
        for _ in 0..6 {
            waktu += Duration::from_millis(16);
            frame(&mut ui, waktu);
        }
        assert!(gulir_node(&ui).bar_opacity() > 0.0, "bar harus muncul");

        // Left alone long enough: the bar fades on its own and the page
        // returns to idle.
        for _ in 0..200 {
            waktu += Duration::from_millis(16);
            frame(&mut ui, waktu);
        }
        assert_eq!(gulir_node(&ui).bar_opacity(), 0.0, "bar harus memudar");
        assert!(ui.is_idle());
    }

    #[test]
    fn router_tidak_pernah_menunjuk_node_mati_setelah_rebuild() {
        // Pressing a scroll-to button rebuilds the list component; if the
        // node's identity did not survive, focus and scroll position would go
        // with it.
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let sebelum = *nodes(ui.tree()).first().expect("ada scroll_view");

        let titik = kotak(&ui, TOMBOL_TENGAH).center();
        klik(&mut ui, titik);
        selesaikan(&mut ui);
        let sesudah = *nodes(ui.tree()).first().expect("ada scroll_view");
        assert_eq!(sebelum, sesudah, "node gulir harus bertahan lintas rebuild");
        assert!(gulir_node(&ui).offset() > 0.0);
    }
}
