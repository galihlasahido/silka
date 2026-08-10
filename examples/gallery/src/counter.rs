//! Demo page: **counter** — living proof that the whole chain works.
//!
//! This page is deliberately as visually simple as possible, because what it
//! proves is not the visuals but the **path**: a single mouse click has to
//! travel through the entire framework and end up as different pixels on
//! screen.
//!
//! ```text
//! klik → hit-test squircle (§3.6)          crates/core: input
//!      → Interactive::event → on_press      crates/core: tree
//!      → Signal::update                     crates/core: signals
//!      → scope "angka" marked dirty          crates/core: signals
//!      → FrameScheduler::request            crates/core: scheduler
//!      → AppRuntime::frame: rebuild ONLY that component  (§2.5)
//!      → view-diff → box constraints + Taffy (§2, §3.4)
//!      → pass paint → Scene (§3.2)
//!      → glyph atlas → wgpu                 crates/renderer
//! ```
//!
//! What is **absent** from this file is the whole point: no hand-assembled
//! `Scene`, no layout arithmetic, not a single color number, and not a single
//! wgpu/cosmic-text type name. All that is written is a Dart-flavored view tree
//! (§2.5) on top of theme tokens (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{button, button_variant, text, ButtonVariant, Fonts};

/// The increment button's name — also used by the tests to find it in the
/// accessibility tree, so what the tests click is **exactly** what a screen
/// reader announces (§3.8).
pub const TOMBOL_TAMBAH: &str = "Tambah";
/// The decrement button's name.
pub const TOMBOL_KURANG: &str = "Kurangi";
/// The reset button's name.
pub const TOMBOL_RESET: &str = "Nol";

/// The page title.
pub const JUDUL: &str = "Counter";

/// The view tree for the whole page — this is what gets handed to `run_app`.
///
/// Read in the root scope: a change of theme or scale factor rebuilds this page
/// in its entirety (every value here is a token), but **the counter is not read
/// here** — that is what makes a click rebuild a single component rather than
/// the page.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution; the logical sizes
    // below do not change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let count = use_signal(|| 0i32);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.body_size * 2.0)
                .weight(FontWeight::SEMIBOLD)
                // Negative tracking at large sizes — an SF habit (§3.6).
                .tracking(-0.02)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Klik tombol di bawah: signal berubah, hanya komponen angka \
                 yang dibangun ulang, dan angkanya benar-benar berganti di layar.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(96.0)),
        ),
        angka(fonts, count),
        kendali(fonts, &t, count),
    ])
    .spacing(t.space(6.0))
    // The whole stack sits centered in the window — the alignment belongs to
    // the layout engine, not to arithmetic on this page (§3.4).
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The big number as **its own component**.
///
/// This is the only place the counter is read, and therefore the only scope
/// marked dirty when a button is pressed (§2.5).
fn angka(fonts: &Fonts, count: Signal<i32>) -> View {
    let fonts = fonts.clone();
    component("angka", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        text(&fonts, count.get().to_string())
            .size(t.typography.body_size * 5.0)
            .weight(FontWeight::BOLD)
            .tracking(-0.03)
            .color(t.color.accent)
            .single_line()
            .into()
    })
}

/// The button row.
///
/// The buttons live in the root scope and do **not** read the counter — their
/// `on_press` closures only write. That is why the button nodes survive
/// unchanged across clicks: what the user's finger is pressing is never rebuilt
/// mid-interaction.
fn kendali(fonts: &Fonts, t: &Theme, count: Signal<i32>) -> View {
    row([
        View::from(button(fonts, t, TOMBOL_TAMBAH).on_press(move || count.update(|n| *n += 1))),
        View::from(
            button_variant(fonts, t, TOMBOL_KURANG, ButtonVariant::Secondary)
                .on_press(move || count.update(|n| *n -= 1)),
        ),
        View::from(
            button_variant(fonts, t, TOMBOL_RESET, ButtonVariant::Secondary)
                .on_press(move || count.set(0)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};
    use silka_theme::{Appearance, Preset};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(720.0, 540.0);
    /// A Retina screen — which also forces the scale factor path to be
    /// exercised.
    const SKALA: f64 = 2.0;

    /// A headless app assembled **exactly the way `run_app` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        let ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height);
        // What the shell does every frame; once is enough here since the test
        // window never moves between monitors.
        ui.env::<Signal<ScaleFactor>>()
            .expect("run_app menitipkan scale factor")
            .set(ScaleFactor(SKALA as f32));
        ui
    }

    /// A deterministic text engine: with no system fonts, test results do not
    /// depend on whichever fonts happen to be installed on the CI machine
    /// (§9.5).
    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// A node's rectangle **according to the accessibility tree**.
    ///
    /// Deliberately via the a11y path: that way the tests click exactly where a
    /// screen reader announces, and the geometry comes from the layout result
    /// (§3.8) — not from coordinates restated here.
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    /// The number currently on screen, read from the a11y tree.
    ///
    /// If this is right, a screen reader announces the same number that is
    /// drawn — both come from the same node.
    fn angka_terbaca(ui: &AppRuntime) -> i32 {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.as_deref()?.parse::<i32>().ok())
            .next()
            .unwrap_or_else(|| panic!("tidak ada label berupa angka:\n{}", pohon.dump()))
    }

    /// One full click through the input layer: move, press, release.
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

    // -- what can be tested without a GPU at all ----------------------------

    #[test]
    fn halaman_menampilkan_teks_dan_tiga_tombol() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        // The text really does become draw commands, not just empty nodes.
        let run: Vec<usize> = ui
            .scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::GlyphRun(r) => Some(r.len()),
                _ => None,
            })
            .collect();
        assert!(
            run.len() >= 5,
            "judul + subjudul + angka + tiga label tombol: {run:?}"
        );
        assert!(
            run.iter().sum::<usize>() > 60,
            "halaman nyaris tanpa glyph: {run:?}"
        );

        // All three buttons are in the a11y tree, clickable, and their hit
        // targets satisfy the HIG.
        let pohon = ui.access_tree();
        for label in [TOMBOL_TAMBAH, TOMBOL_KURANG, TOMBOL_RESET] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, silka_core::access::AccessRole::Button);
            assert!(e
                .node
                .actions
                .contains(silka_core::access::AccessActions::CLICK));
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_menaikkan_angka_dan_hanya_komponen_angka_yang_dibangun_ulang() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);

        let tambah = kotak(&ui, TOMBOL_TAMBAH).center();
        klik(&mut ui, tambah);
        assert!(
            !ui.is_idle(),
            "klik harus menjadwalkan tepat satu frame lewat signal"
        );

        let laporan = ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);
        assert_eq!(
            laporan.rebuilt, 1,
            "hanya komponen angka yang membaca pencacah"
        );
        assert_eq!(laporan.diff.created, 0, "tidak ada node yang lahir ulang");
        assert_eq!(laporan.diff.removed, 0);
        assert!(ui.is_idle());

        // The other buttons write the same counter, from the same scope.
        for _ in 0..3 {
            let p = kotak(&ui, TOMBOL_TAMBAH).center();
            klik(&mut ui, p);
            ui.frame();
        }
        assert_eq!(angka_terbaca(&ui), 4);

        let p = kotak(&ui, TOMBOL_KURANG).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 3);

        let p = kotak(&ui, TOMBOL_RESET).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);
    }

    #[test]
    fn klik_di_luar_tombol_tidak_mengubah_apa_pun() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        // Top-left corner of the window: far from the centered stack.
        klik(&mut ui, Point::new(4.0, 4.0));
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);
    }

    #[test]
    fn keyboard_bisa_mengaktifkan_tombol_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        // Tab moves focus to the first button, Space activates it — the
        // keyboard is not a second-class citizen (`KOMPONEN.md` DoD).
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);
    }

    #[test]
    fn warna_dan_ukuran_selalu_datang_dari_token() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                let warna: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                for w in warna {
                    assert!(
                        w == t.color.label
                            || w == t.color.secondary_label
                            || w == t.color.accent
                            || w == t.color.on_accent,
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }

                // The primary button's background is a token too, in both
                // presets.
                let kotak_tombol = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) if q.background == t.color.accent => Some(q.clone()),
                        _ => None,
                    })
                    .count();
                assert_eq!(kotak_tombol, 1, "tepat satu tombol primary");
            }
        }
    }

    #[test]
    fn ganti_theme_membangun_ulang_halaman_dan_angka_ikut_ganti_warna() {
        let f = fonts();
        let terang = Theme::cupertino(Appearance::Light);
        let mut ui = ui(terang, &f);
        ui.frame();
        let p = kotak(&ui, TOMBOL_TAMBAH).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);

        let gelap = Theme::cupertino(Appearance::Dark);
        ui.env::<Signal<Theme>>()
            .expect("theme dititipkan")
            .set(gelap);
        ui.set_clear_color(gelap.color.background);
        ui.frame();

        // State survives the theme rebuild — only the tokens change.
        assert_eq!(angka_terbaca(&ui), 1);
        assert!(ui
            .scene()
            .commands()
            .iter()
            .any(|c| matches!(c, Command::GlyphRun(r) if r.color == gelap.color.accent)));
    }

    // -- pixel proof: what changes is not just state, but the screen --------

    /// Count the pixels that are **not** the background color inside a logical
    /// rectangle, and hash its contents.
    ///
    /// The hash is FNV-1a over the region's raw bytes — enough to answer the
    /// one question that matters: "is this part of the screen actually
    /// different?"
    fn cuplik(img: &Rgba8Image, wilayah: Rect, latar: silka_paint::Color) -> (u32, u64) {
        let f = |v: f32| (v as f64 * SKALA).round().max(0.0) as u32;
        let mut n = 0u32;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for y in f(wilayah.min_y())..f(wilayah.max_y()).min(img.height()) {
            for x in f(wilayah.min_x())..f(wilayah.max_x()).min(img.width()) {
                let p = img.pixel(x, y);
                for c in p {
                    hash ^= c as u64;
                    hash = hash.wrapping_mul(0x100_0000_01b3);
                }
                let jauh = |c: u8, token: f32| (c as f32 - token * 255.0).abs() > 24.0;
                if jauh(p[0], latar.r) || jauh(p[1], latar.g) || jauh(p[2], latar.b) {
                    n += 1;
                }
            }
        }
        (n, hash)
    }

    /// **The most valuable integration test in the repo**: a single click
    /// simulated through the input layer must end up as different pixels in a
    /// texture the GPU rendered through **exactly the same** path as the window
    /// (pipeline, sRGB format, blending, glyph atlas).
    ///
    /// Every other test in this file stops on the CPU side and would stay green
    /// even with a blank screen; this one cannot.
    #[test]
    fn klik_mengubah_piksel_angka_di_layar() {
        let Ok(gpu) = Gpu::headless() else {
            eprintln!("dilewati: tidak ada GPU untuk render headless");
            return;
        };

        let f = fonts();
        let theme = Theme::cupertino(Appearance::Dark);
        let mut ui = ui(theme, &f);
        ui.frame();

        let mut target = OffscreenTarget::new(&gpu, SurfaceGeometry::from_logical(VIEWPORT, SKALA))
            .expect("target headless");
        let gambar = |ui: &AppRuntime, target: &mut OffscreenTarget| -> Rgba8Image {
            f.with(|mesin| target.render_with_glyphs(&gpu, ui.scene(), mesin))
                .expect("render halaman counter")
        };

        // A horizontal band as tall as the number: only the number falls
        // inside it, and its rectangle comes from the layout result (via the
        // a11y tree), not from coordinates restated here.
        let angka0 = kotak(&ui, "0");
        let pita = Rect::new(
            0.0,
            angka0.min_y(),
            VIEWPORT.width,
            angka0.size.height.max(1.0),
        );

        let sebelum = gambar(&ui, &mut target);
        let (n0, h0) = cuplik(&sebelum, pita, theme.color.background);
        assert!(
            n0 > 200,
            "angka \"0\" nyaris tidak tergambar: hanya {n0} piksel bukan-latar"
        );

        // Negative control: an empty band inside the page padding must be
        // exactly zero, proving the sampling threshold does not pass by
        // accident.
        let kosong = Rect::new(0.0, 0.0, VIEWPORT.width, theme.space(4.0));
        assert_eq!(
            cuplik(&sebelum, kosong, theme.color.background).0,
            0,
            "ambang sampel salah: pita kosong sudah punya piksel bukan-latar"
        );

        // One click through the input layer.
        let tambah = kotak(&ui, TOMBOL_TAMBAH).center();
        klik(&mut ui, tambah);
        assert!(!ui.is_idle());
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);

        let sesudah = gambar(&ui, &mut target);
        let (n1, h1) = cuplik(&sesudah, pita, theme.color.background);
        assert!(n1 > 100, "angka \"1\" tidak tergambar: {n1} piksel");
        assert_ne!(
            h0, h1,
            "layar tidak berubah setelah klik — angka {n0} vs {n1} piksel"
        );

        // Going back to zero must restore **exactly the same** pixels: proof
        // that what changed is the content, not something accumulating frame
        // after frame.
        let nol = kotak(&ui, TOMBOL_RESET).center();
        klik(&mut ui, nol);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);
        let lagi = gambar(&ui, &mut target);
        let (n2, h2) = cuplik(&lagi, pita, theme.color.background);
        assert_eq!((n0, h0), (n2, h2), "kembali ke 0 harus identik dengan awal");
    }
}
