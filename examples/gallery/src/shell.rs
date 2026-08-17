//! The gallery **shell**: the chrome that turns a folder of demo pages into an
//! application (REKOMENDASI §9.9).
//!
//! Three things live here, and all three are what makes the gallery useful as
//! the day-to-day visual QA tool rather than a pile of examples:
//!
//! 1. **A sidebar listing every component** ([`crate::catalog`]), grouped by
//!    the tiers of `KOMPONEN.md`, so "what does this framework actually have?"
//!    is answered by looking, not by reading source.
//! 2. **A live preset and appearance switcher.** Cupertino ⇄ Tailwind and
//!    light ⇄ dark ⇄ system, applied without restarting: a token regression is
//!    two clicks away instead of two rebuilds away (§2.7).
//! 3. **A reduced-motion switch**, so the "respects reduce motion" line of
//!    every component's Definition of Done can be checked by hand.
//!
//! ## Why this file wires the runtime itself instead of calling `run_app`
//!
//! [`silka_platform::run_app`] pushes the window's theme into the
//! `Signal<Theme>` **every frame**: the shell owns the theme, the application
//! reads it. That is right for an ordinary application and wrong for exactly
//! one kind of application — this one, where changing the theme *is* the
//! feature. So the gallery assembles the same runtime by hand
//! ([`silka_platform::headless_app`] plus the same four callbacks `run_app`
//! installs) and reverses the direction of that one value: the window
//! **announces** the OS appearance, and the gallery decides what to do with it
//! ([`tema_berikut`]).
//!
//! Acknowledged debt: this is a copy of `run_app`'s frame wiring, so a change
//! there has to be repeated here. The proper fix is a `WindowConfig` option
//! along the lines of "the application owns the theme"; that touches the
//! platform crate and is deliberately deferred rather than smuggled in with a
//! gallery change.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::animation::{Motion, Tick};
use silka_core::app::{component, AppRuntime, BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, RenderTree};
use silka_core::view::{column, constrained, expanded, row, View};
use silka_paint::Insets;
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_text::FontWeight;
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::tabs::{tab, tabs_in, TabsVariant};
use silka_widgets::{
    button_variant_in, scroll_view_in, spacer, switch_in, text_in, ButtonVariant, Fonts,
};

use crate::catalog::{Halaman, Kelompok};

/// Width of the navigation sidebar, in spacing steps.
const LEBAR_SISI: f32 = 46.0;

/// The a11y name of the sidebar — the tests find it by this name, which is
/// also what a screen reader announces (§3.8).
pub const NAMA_SISI: &str = "Daftar komponen";
/// The a11y name of the preset switcher.
pub const NAMA_PRESET: &str = "Preset";
/// The a11y name of the appearance switcher.
pub const NAMA_TAMPILAN: &str = "Tampilan";
/// The label of the reduced-motion switch.
pub const NAMA_GERAK: &str = "Kurangi gerak";
/// The brand shown at the top left.
pub const MEREK: &str = "silka";

// ---------------------------------------------------------------------------
// Shell state (kept in `Env`, so any page can read it too)
// ---------------------------------------------------------------------------

/// How the gallery picks between light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModeTampilan {
    /// Follow the OS — the default, and the setting that makes a live dark
    /// mode change visible while the window stays open.
    #[default]
    Sistem,
    /// Pinned to light, whatever the OS says.
    Terang,
    /// Pinned to dark, whatever the OS says.
    Gelap,
}

impl ModeTampilan {
    /// Every mode, in the order of the segmented control.
    pub const SEMUA: [ModeTampilan; 3] = [
        ModeTampilan::Sistem,
        ModeTampilan::Terang,
        ModeTampilan::Gelap,
    ];

    /// The label in the segmented control.
    pub fn judul(self) -> &'static str {
        match self {
            ModeTampilan::Sistem => "Sistem",
            ModeTampilan::Terang => "Terang",
            ModeTampilan::Gelap => "Gelap",
        }
    }

    /// Its index in [`ModeTampilan::SEMUA`].
    pub fn indeks(self) -> usize {
        ModeTampilan::SEMUA
            .iter()
            .position(|m| *m == self)
            .unwrap_or(0)
    }

    /// The appearance this mode pins to, or `None` when it follows the OS.
    pub fn appearance(self) -> Option<Appearance> {
        match self {
            ModeTampilan::Sistem => None,
            ModeTampilan::Terang => Some(Appearance::Light),
            ModeTampilan::Gelap => Some(Appearance::Dark),
        }
    }
}

/// The reduced-motion override offered by the shell.
///
/// A newtype rather than a bare `bool` because [`silka_core::app::Env`] is
/// keyed by type: `Signal<bool>` would collide with the next boolean anyone
/// puts in there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GerakDikurangi(pub bool);

/// The theme the next frame should use.
///
/// Pure on purpose: this is the whole "who owns the theme" decision, and it is
/// tested directly rather than through a window that cannot exist in CI.
/// The preset always survives — only the appearance is decided here.
pub fn tema_berikut(sekarang: Theme, mode: ModeTampilan, os: Appearance) -> Theme {
    sekarang.with_appearance(mode.appearance().unwrap_or(os))
}

// ---------------------------------------------------------------------------
// Animation driver
// ---------------------------------------------------------------------------

/// One tick for everything that moves in the gallery.
///
/// Three sources, because animation belongs to whoever owns the node: the
/// widget catalogue, the chart crate, and the gallery's own spring playground.
/// The application still calls a single function once per frame (§3.5).
pub fn maju(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    silka_widgets::advance(tree, tick)
        | silka_chart::advance(tree, tick)
        | crate::spring::advance(tree, tick)
}

// ---------------------------------------------------------------------------
// Runtime assembly
// ---------------------------------------------------------------------------

/// The gallery's `AppRuntime`, with the shell's own values in [`Env`].
///
/// Shared by the window and by the tests, so a test can never accidentally
/// exercise a different application than the one that ships.
///
/// [`Env`]: silka_core::app::Env
pub fn aplikasi(tema: Theme, fonts: &Fonts, awal: Halaman, solo: bool) -> AppRuntime {
    let untuk_view = fonts.clone();
    headless_app(tema, move |cx| {
        if solo {
            solo_view(cx, &untuk_view, awal)
        } else {
            kerangka(cx, &untuk_view, awal)
        }
    })
    .with_env(|rt| rt.signal(ModeTampilan::default()))
    .with_env(|rt| rt.signal(GerakDikurangi::default()))
}

/// Open the window and run the gallery.
///
/// `solo` drops the chrome and shows a single page filling the window — the
/// shape wanted for pixel-level QA, where the sidebar would only be noise.
pub fn jalankan(
    config: WindowConfig,
    tema: Theme,
    fonts: Fonts,
    awal: Halaman,
    solo: bool,
) -> Result<(), PlatformError> {
    let ui = aplikasi(tema, &fonts, awal, solo);

    // Read the handles out **before** the runtime moves into the closures:
    // afterwards it lives behind a `RefCell` that the frame callback borrows.
    let mode = ui
        .env::<Signal<ModeTampilan>>()
        .expect("shell menitipkan mode tampilan");
    let tema_sig = ui
        .env::<Signal<Theme>>()
        .expect("headless_app menitipkan Signal<Theme>");
    let gerak = ui
        .env::<Signal<GerakDikurangi>>()
        .expect("shell menitipkan mode gerak");
    let skala = ui.env::<Signal<ScaleFactor>>();

    let app = Rc::new(RefCell::new(ui));
    let untuk_frame = app.clone();
    let untuk_input = app.clone();
    let untuk_access = app;

    // Only a *change* flips the animation driver: asserting it every frame
    // would fight whoever wires the OS "reduce motion" setting later on.
    let mut gerak_terakhir = GerakDikurangi::default();

    config
        // Without this line the `GlyphRun` commands carry no bitmaps and every
        // page renders blank — the atlas is what crosses over to the GPU.
        .glyphs(fonts.shared())
        // The same sentence for bitmaps: without it every `Command::Image`
        // draws nothing, so the icons on the layout page would simply be gone.
        .images(silka_widgets::active_images().shared())
        .on_frame(move |ctx| {
            let mut ui = untuk_frame.borrow_mut();
            ui.resize(ctx.size());

            // The window announces the OS appearance; the gallery decides.
            // This is the one line that differs from `run_app`, and it is the
            // reason the preset switcher can exist at all.
            tema_sig.set_if_changed(tema_berikut(
                tema_sig.get(),
                mode.get(),
                ctx.theme().appearance,
            ));
            ui.set_clear_color(tema_sig.get().color.background);

            // Text is rasterised at the real screen resolution; a window
            // dragged to another monitor writes this signal and only the
            // components that read it are rebuilt (§3.3).
            if let Some(s) = skala {
                s.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            ui.set_vsync(ctx.vsync());

            let g = gerak.get();
            if g != gerak_terakhir {
                gerak_terakhir = g;
                let _ = ui.set_motion(Motion::from_reduced(g.0));
            }

            // Springs are advanced **before** the frame: the value that moves
            // becomes this frame's value, not the next frame's (§3.5).
            let _ = ui.animate(maju);
            ui.frame();

            // The only way a next frame happens: something is still dirty.
            if !ui.is_idle() {
                ctx.request_animation_frame();
            }
            ui.scene().clone()
        })
        .on_input(move |event| untuk_input.borrow_mut().dispatch(event))
        .on_access(move || untuk_access.borrow().access_tree())
        .run()
}

// ---------------------------------------------------------------------------
// The view tree
// ---------------------------------------------------------------------------

/// A single page filling the window, with no chrome (`--solo`).
fn solo_view(cx: &BuildCtx, fonts: &Fonts, halaman: Halaman) -> View {
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());
    // Icons are coverage masks tied to a pixel grid, exactly like glyphs, so
    // the bitmap atlas needs the same number (§3.3).
    silka_widgets::active_images().set_scale_factor(dpi.get());
    halaman.view(cx, fonts)
}

/// The whole shell: top bar, sidebar, content.
///
/// The root scope reads the **theme** (every piece of chrome is made of
/// tokens, so a preset change genuinely does rebuild all of it) but not the
/// selected page — that lives one level down, so switching pages rebuilds the
/// sidebar and the content area only (§2.5).
fn kerangka(cx: &BuildCtx, fonts: &Fonts, awal: Halaman) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());
    // Icons are coverage masks tied to a pixel grid, exactly like glyphs, so
    // the bitmap atlas needs the same number (§3.3).
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let halaman = use_signal(|| awal);

    let badan = row([sisi(fonts, halaman), isi(fonts, halaman)])
        // The sidebar is as tall as the window, not as tall as its buttons.
        .cross(CrossAlign::Stretch);

    column([bilah_atas(fonts), View::from(expanded(badan))])
        .cross(CrossAlign::Stretch)
        .background(t.color.background)
        .into()
}

/// The top bar: brand on the reading-start side, switchers on the other.
///
/// Its own component, so flipping a switcher does not rebuild the page that
/// happens to be open — only the bar and whatever actually reads the theme.
fn bilah_atas(fonts: &Fonts) -> View {
    let fonts = fonts.clone();
    component("bilah-atas", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let tema_sig: Signal<Theme> = cx.expect_env();
        let mode: Signal<ModeTampilan> = cx.expect_env();
        let gerak: Signal<GerakDikurangi> = cx.expect_env();

        let preset_aktif = usize::from(t.preset == Preset::Tailwind);
        let pemilih_preset = tabs_in(&fonts, &t, [tab("Cupertino"), tab("Tailwind")])
            .variant(TabsVariant::Segmented)
            .selected(preset_aktif)
            .label(NAMA_PRESET)
            .on_select(move |i| {
                let preset = if i == 0 {
                    Preset::Cupertino
                } else {
                    Preset::Tailwind
                };
                tema_sig.update(|t| *t = t.with_preset(preset));
            });

        let pemilih_tampilan = tabs_in(&fonts, &t, ModeTampilan::SEMUA.map(|m| tab(m.judul())))
            .variant(TabsVariant::Segmented)
            .selected(mode.get().indeks())
            .label(NAMA_TAMPILAN)
            .on_select(move |i| {
                let m = ModeTampilan::SEMUA[i];
                mode.set(m);
                // Applied here as well as in the frame callback so the
                // switcher also works in a headless test, where there is no
                // window to announce anything.
                if let Some(a) = m.appearance() {
                    tema_sig.update(|t| *t = t.with_appearance(a));
                }
            });

        // The switch only writes the signal; the animation driver itself is
        // flipped by the frame callback, because `set_motion` belongs to the
        // runtime and a view callback may only touch signals (§2.5).
        let sakelar_gerak = switch_in(&fonts, &t, NAMA_GERAK)
            .on(gerak.get().0)
            .on_change(move |v| gerak.set(GerakDikurangi(v)));

        row([
            text_in(&fonts, MEREK)
                .size(t.typography.headline.size)
                .weight(FontWeight::BOLD)
                .tracking(t.typography.headline.tracking)
                .color(t.color.label)
                .single_line()
                .into(),
            text_in(&fonts, "Gallery")
                .size(t.typography.headline.size)
                .color(t.color.tertiary_label)
                .single_line()
                .into(),
            // The spacer that pushes the switchers to the far side; it is a
            // flex child with zero size, so the layout engine owns the gap
            // rather than a hand-computed number.
            View::from(spacer()),
            View::from(pemilih_preset),
            View::from(pemilih_tampilan),
            View::from(sakelar_gerak),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .padding(Insets::symmetric(t.space(4.0), t.space(2.0)))
        .background(t.color.surface)
        .border(t.space(0.25), t.color.separator)
        .into()
    })
}

/// The navigation sidebar: one button per catalogue entry, grouped by tier.
///
/// The selected entry is a `primary` button and the rest are `ghost` — the
/// same trick a source list uses, without inventing a widget for it.
fn sisi(fonts: &Fonts, halaman: Signal<Halaman>) -> View {
    let fonts = fonts.clone();
    component("navigasi", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let aktif = halaman.get();

        let mut anak: Vec<View> = Vec::with_capacity(Halaman::SEMUA.len() + Kelompok::SEMUA.len());
        for kelompok in Kelompok::SEMUA {
            let isi_kelompok: Vec<Halaman> = Halaman::SEMUA
                .into_iter()
                .filter(|h| h.kelompok() == kelompok)
                .collect();
            if isi_kelompok.is_empty() {
                continue;
            }
            anak.push(
                text_in(&fonts, kelompok.judul())
                    .size(t.typography.caption1.size)
                    .weight(FontWeight::SEMIBOLD)
                    .tracking(t.typography.caption1.tracking)
                    .color(t.color.tertiary_label)
                    .single_line()
                    .into(),
            );
            for h in isi_kelompok {
                let variant = if h == aktif {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                };
                anak.push(
                    button_variant_in(&fonts, &t, h.judul(), variant)
                        .key(h.slug())
                        .on_press(move || halaman.set(h))
                        .into(),
                );
            }
        }

        let daftar = column(anak)
            .spacing(t.space(1.0))
            .cross(CrossAlign::Stretch)
            .padding(Insets::all(t.space(3.0)));

        constrained(
            // A fixed width, free height: the height comes from the row above,
            // and the scroll axis must be bounded (the same rule as Flutter's).
            BoxConstraints::new(t.space(LEBAR_SISI), t.space(LEBAR_SISI), 0.0, f32::INFINITY),
            scroll_view_in(&t, daftar)
                .label(NAMA_SISI)
                .background(t.color.surface),
        )
        .into()
    })
}

/// The content area: the page that is currently selected.
///
/// Each page is built inside a component **keyed by its slug**, so switching
/// pages drops the old scope with all of its state instead of handing the
/// next page a drawer full of someone else's signals.
fn isi(fonts: &Fonts, halaman: Signal<Halaman>) -> View {
    let fonts = fonts.clone();
    let luar = component("isi", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let h = halaman.get();
        let untuk_halaman = fonts.clone();
        let dalam = component(h.slug(), move |cx| h.view(cx, &untuk_halaman));

        if h.gulir_sendiri() {
            dalam
        } else {
            // Everything else gets a scrolling container, so a long catalogue
            // page is still reachable in a small window instead of being cut
            // off at the bottom edge.
            scroll_view_in(&t, dalam).label(h.judul()).into()
        }
    });
    expanded(luar).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(1280.0, 860.0);

    fn fonts() -> Fonts {
        // No system fonts: results must not depend on which fonts the machine
        // running the tests happens to have installed (§9.5).
        Fonts::bundled_only()
    }

    fn ui(tema: Theme, fonts: &Fonts) -> AppRuntime {
        let mut ui =
            aplikasi(tema, fonts, Halaman::AWAL, false).sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        ui
    }

    /// A node's rectangle **according to the a11y tree** — the tests click
    /// exactly where a screen reader announces (§3.8).
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn ada_label(ui: &AppRuntime, label: &str) -> bool {
        ui.access_tree().find_label(label).is_some()
    }

    /// One full tap at point `p`, followed by the frame it schedules.
    fn klik(ui: &mut AppRuntime, p: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
        ui.frame();
    }

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    #[test]
    fn sidebar_menampilkan_seluruh_katalog() {
        let f = fonts();
        let ui = ui(tema(), &f);
        let pohon = ui.access_tree();
        let label: Vec<&str> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Button)
            .filter_map(|e| e.node.label.as_deref())
            .collect();
        for h in Halaman::SEMUA {
            assert!(
                label.contains(&h.judul()),
                "komponen '{}' tidak punya tombol navigasi — galeri yang tidak \
                 mendaftarkan komponennya adalah galeri yang bohong",
                h.judul()
            );
        }
    }

    #[test]
    fn klik_navigasi_mengganti_halaman() {
        let f = fonts();
        let mut ui = ui(tema(), &f);
        // The counter page is the smallest page with an unmistakable label.
        let tombol = kotak(&ui, Halaman::Counter.judul());
        klik(&mut ui, tombol.center());

        assert!(
            ada_label(&ui, crate::counter::TOMBOL_TAMBAH),
            "halaman counter tidak terbuka setelah tombolnya diklik"
        );
    }

    /// True when the scene paints a box in exactly this colour.
    ///
    /// The shell's own background quad is what the eye actually sees; the
    /// window's clear colour is set by the frame callback, which does not exist
    /// in a headless test.
    fn menggambar_latar(ui: &AppRuntime, warna: silka_paint::Color) -> bool {
        ui.scene().commands().iter().any(|c| match c {
            silka_paint::Command::Quad(q) => q.background == warna,
            _ => false,
        })
    }

    #[test]
    fn pemilih_preset_mengganti_token_seluruh_aplikasi() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        assert!(menggambar_latar(
            &ui,
            Theme::cupertino(Appearance::Light).color.background
        ));

        let tab_tailwind = kotak(&ui, "Tailwind");
        klik(&mut ui, tab_tailwind.center());

        let tema: Signal<Theme> = ui.env().expect("Signal<Theme>");
        assert_eq!(tema.get().preset, Preset::Tailwind);
        assert!(
            menggambar_latar(&ui, Theme::tailwind(Appearance::Light).color.background),
            "latar belakang harus ikut preset, bukan menempel di nilai lama"
        );
    }

    #[test]
    fn pemilih_tampilan_mengunci_gelap() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        let tab_gelap = kotak(&ui, ModeTampilan::Gelap.judul());
        klik(&mut ui, tab_gelap.center());

        let mode: Signal<ModeTampilan> = ui.env().expect("Signal<ModeTampilan>");
        assert_eq!(mode.get(), ModeTampilan::Gelap);
        let tema: Signal<Theme> = ui.env().expect("Signal<Theme>");
        assert_eq!(tema.get().appearance, Appearance::Dark);
    }

    #[test]
    fn mode_sistem_ikut_os_mode_terkunci_tidak() {
        let terang = Theme::cupertino(Appearance::Light);
        // Following the OS: a dark-mode change on the OS lands.
        assert_eq!(
            tema_berikut(terang, ModeTampilan::Sistem, Appearance::Dark).appearance,
            Appearance::Dark
        );
        // Pinned: the same OS change is ignored.
        assert_eq!(
            tema_berikut(terang, ModeTampilan::Terang, Appearance::Dark).appearance,
            Appearance::Light
        );
        // And the preset always survives — this is the bug that would make the
        // preset switcher useless: the OS resets it every frame.
        let tailwind = Theme::tailwind(Appearance::Dark);
        assert_eq!(
            tema_berikut(tailwind, ModeTampilan::Sistem, Appearance::Light).preset,
            Preset::Tailwind
        );
    }

    #[test]
    fn sakelar_gerak_mengubah_driver_animasi() {
        let f = fonts();
        let mut ui = ui(tema(), &f);
        let sakelar = kotak(&ui, NAMA_GERAK);
        klik(&mut ui, sakelar.center());

        let gerak: Signal<GerakDikurangi> = ui.env().expect("Signal<GerakDikurangi>");
        assert!(gerak.get().0, "sakelar tidak menulis signal gerak");
    }

    #[test]
    fn setiap_halaman_bisa_dibangun_dan_menghasilkan_gambar() {
        let f = fonts();
        for h in Halaman::SEMUA {
            let mut ui = aplikasi(tema(), &f, h, true).sized(VIEWPORT.width, VIEWPORT.height);
            ui.frame();
            assert!(
                !ui.scene().is_empty(),
                "halaman '{}' tidak menggambar apa pun",
                h.judul()
            );
        }
    }

    // -- layout: the shell has to be *arranged*, not merely present ---------

    #[test]
    fn kerangka_tersusun_bilah_atas_sidebar_lalu_isi() {
        let f = fonts();
        let t = tema();
        let mut ui = ui(t, &f);

        let sisi = kotak(&ui, NAMA_SISI);
        let preset = kotak(&ui, NAMA_PRESET);

        // The sidebar hugs the reading-start edge and keeps the width the
        // catalogue was laid out for.
        assert_eq!(sisi.min_x(), 0.0, "sidebar tidak menempel di tepi kiri");
        assert!(
            (sisi.size.width - t.space(LEBAR_SISI)).abs() < 0.5,
            "lebar sidebar {} bukan {}",
            sisi.size.width,
            t.space(LEBAR_SISI)
        );
        // …and it is as tall as what is left of the window, not as tall as its
        // buttons.
        assert!(
            sisi.max_y() >= VIEWPORT.height - 0.5,
            "sidebar berhenti di {} padahal jendela setinggi {}",
            sisi.max_y(),
            VIEWPORT.height
        );

        // The top bar really is above the body, and its switchers sit on the
        // far side of the window.
        assert!(
            preset.max_y() <= sisi.min_y() + 0.5,
            "pemilih preset ({}) tidak berada di atas sidebar ({})",
            preset.max_y(),
            sisi.min_y()
        );
        assert!(
            preset.min_x() > VIEWPORT.width * 0.5,
            "pemilih preset seharusnya di sisi kanan bilah, bukan di {}",
            preset.min_x()
        );

        // Every navigation button lives inside the sidebar — the arrangement
        // no unit test would notice if the row above collapsed.
        for h in Halaman::SEMUA {
            let b = kotak(&ui, h.judul());
            assert!(
                b.min_x() >= sisi.min_x() - 0.5 && b.max_x() <= sisi.max_x() + 0.5,
                "tombol '{}' keluar dari sidebar: {b:?} vs {sisi:?}",
                h.judul()
            );
        }

        // And the page opens **beside** the sidebar, never underneath it.
        let nav_counter = kotak(&ui, Halaman::Counter.judul()).center();
        klik(&mut ui, nav_counter);
        let tambah = kotak(&ui, crate::counter::TOMBOL_TAMBAH);
        assert!(
            tambah.min_x() >= sisi.max_x() - 0.5,
            "isi halaman menimpa sidebar: {} < {}",
            tambah.min_x(),
            sisi.max_x()
        );
        assert!(
            tambah.min_y() >= preset.max_y() - 0.5,
            "isi halaman menimpa bilah atas"
        );
    }

    // -- pixel proof: the shell is not just a tree, it is on the screen -----

    /// Count the pixels that are **not** the background colour in a region,
    /// and hash it: enough to answer "did this part of the screen change?".
    fn cuplik(
        img: &silka_renderer::Rgba8Image,
        wilayah: Rect,
        latar: silka_paint::Color,
        skala: f64,
    ) -> (u32, u64) {
        let f = |v: f32| (v as f64 * skala).round().max(0.0) as u32;
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

    /// The test that cannot stay green with a blank window: the gallery is
    /// rendered through the same GPU path the window uses, and switching preset
    /// or appearance has to change the **pixels** — not merely a signal.
    #[test]
    fn ganti_preset_dan_gelap_mengubah_piksel_di_layar() {
        let Ok(gpu) = silka_renderer::Gpu::headless() else {
            eprintln!("dilewati: tidak ada GPU untuk render headless");
            return;
        };

        const SKALA: f64 = 2.0;
        let f = fonts();
        let terang = Theme::cupertino(Appearance::Light);
        let mut ui = ui(terang, &f);
        // The scale factor the window would report; without it the glyphs are
        // rasterised for the wrong resolution.
        if let Some(s) = ui.env::<Signal<ScaleFactor>>() {
            s.set(ScaleFactor(SKALA as f32));
        }
        ui.frame();

        let mut target = silka_renderer::OffscreenTarget::new(
            &gpu,
            silka_renderer::SurfaceGeometry::from_logical(VIEWPORT, SKALA),
        )
        .expect("target headless");
        let gambar = |ui: &AppRuntime, target: &mut silka_renderer::OffscreenTarget| {
            f.with(|mesin| target.render_with_glyphs(&gpu, ui.scene(), mesin))
                .expect("render kerangka galeri")
        };

        let sisi = kotak(&ui, NAMA_SISI);
        let sebelum = gambar(&ui, &mut target);
        let (n0, h0) = cuplik(&sebelum, sisi, terang.color.surface, SKALA);
        assert!(
            n0 > 500,
            "sidebar nyaris kosong di layar: hanya {n0} piksel bukan-latar"
        );

        // Negative control: a band above the window has nothing in it, so the
        // sampling threshold cannot be passing by accident.
        let kosong = Rect::new(0.0, -20.0, VIEWPORT.width, 10.0);
        assert_eq!(cuplik(&sebelum, kosong, terang.color.surface, SKALA).0, 0);

        let tab_tailwind = kotak(&ui, "Tailwind").center();
        klik(&mut ui, tab_tailwind);
        let tailwind = gambar(&ui, &mut target);
        let (_, h1) = cuplik(&tailwind, sisi, terang.color.surface, SKALA);
        assert_ne!(h0, h1, "ganti preset tidak mengubah satu piksel pun");

        let tab_gelap = kotak(&ui, ModeTampilan::Gelap.judul()).center();
        klik(&mut ui, tab_gelap);
        let gelap = gambar(&ui, &mut target);
        let (_, h2) = cuplik(&gelap, sisi, terang.color.surface, SKALA);
        assert_ne!(h1, h2, "mode gelap tidak mengubah satu piksel pun");
    }

    #[test]
    fn solo_tidak_membawa_kerangka() {
        let f = fonts();
        let mut ui =
            aplikasi(tema(), &f, Halaman::Counter, true).sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        assert!(
            !ada_label(&ui, NAMA_SISI),
            "mode solo seharusnya tanpa sidebar"
        );
    }
}
