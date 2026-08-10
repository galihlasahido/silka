//! Demo page: **switch / toggle** (`KOMPONEN.md` Tier 2).
//!
//! What this page shows off is the component's Definition of Done, item by
//! item, in a form you can **see and try by hand** — not one claimed in a
//! comment:
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Correct in both presets | `--preset cupertino` (52×32 track, squircle corners) vs `--preset tailwind` (44×24, arc) |
//! | Dark mode | `--appearance dark` / `light`, or follow the OS |
//! | **Spring drag** (this component's special note) | Press the thumb and **drag**: it tracks your finger 1:1, and the track color flips exactly as it crosses the midpoint |
//! | Velocity handoff → spring | Fling the thumb from a third of the way across: the fling direction beats the position, and the spring continues your finger's velocity — it does not restart from zero |
//! | Retargetable spring | Click twice quickly: the thumb reverses from where it currently is, it does not jump |
//! | Hover / press | The thumb stretches slightly while held (the iOS feel) and the track color shifts via the hover/pressed tokens |
//! | Keyboard + focus ring | Tab around; **Space** flips it, left/right arrows (and Home/End) set an explicit value; the focus ring **grows** |
//! | Hit target ≥ 44pt | The track is 32pt/24pt tall, but the whole row — label included — is clickable |
//! | AccessKit nodes | VoiceOver announces "switch, on/off" from the same node that is drawn |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the bounce goes away, the thumb still **slides** (motion that explains must not be removed) |
//!
//! What is **absent** from this file is the whole point: no hand-assembled
//! `Scene`, no layout arithmetic, and not a single color number — everything is
//! a token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{switch, switch_only, text, Fonts};

/// The page title.
pub const JUDUL: &str = "Switch";
/// The parent switch's name: turning it on turns everything below it off.
pub const MODE_PESAWAT: &str = "Mode pesawat";
/// The name of each radio switch.
pub const RADIO: [&str; 3] = ["Wi-Fi", "Bluetooth", "Data seluler"];
/// The name of the switch deliberately disabled in the off state.
pub const MATI: &str = "Tidak tersedia di paket ini";
/// The name of the switch deliberately disabled in the on state.
pub const TERKUNCI: &str = "Wajib menyala";
/// The name of the switch with no visible label (its a11y name still
/// exists).
pub const TANPA_LABEL: &str = "Sinkronkan baris pertama";

/// How many radios are on — used by the summary row **and** by the tests.
pub fn menyala(radio: &[bool]) -> usize {
    radio.iter().filter(|v| **v).count()
}

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
///
/// The title and the description are read in the root scope; **the switch
/// values are not**, so a single tap rebuilds one component rather than the
/// page (§2.5).
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let radio = use_signal(|| [true, false, true]);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                // Negative tracking at large sizes — an SF habit (§3.6).
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Jangan cuma diklik — tekan thumb-nya lalu seret. Ia mengikuti \
                 jari 1:1, warnanya berganti tepat saat melewati tengah, dan saat \
                 dilepas kecepatan jari diteruskan ke spring, bukan dibuang.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        kelompok(fonts, radio),
        mati(fonts, &t),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The "airplane mode" + three radios group, as **its own component**.
///
/// This is the only place the values are read, and therefore the only scope
/// marked dirty when a switch is flipped.
fn kelompok(fonts: &Fonts, radio: Signal<[bool; RADIO.len()]>) -> View {
    let fonts = fonts.clone();
    component("sakelar", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let nilai = radio.get();
        // Airplane mode is **derived** from the data, like a parent checkbox:
        // on means every radio is off.
        let pesawat = menyala(&nilai) == 0;

        let mut anak: Vec<View> = Vec::with_capacity(RADIO.len() + 2);
        anak.push(
            switch(&fonts, &t, MODE_PESAWAT)
                .key("pesawat")
                .on(pesawat)
                // Turned on = every radio off; turned off = they all come
                // back.
                .on_change(move |nyala| radio.set([!nyala; RADIO.len()]))
                .into(),
        );
        for (i, label) in RADIO.into_iter().enumerate() {
            anak.push(
                switch(&fonts, &t, label)
                    .key(label)
                    .on(nilai[i])
                    .on_change(move |v| {
                        radio.update(|semua| semua[i] = v);
                    })
                    .into(),
            );
        }
        anak.push(
            text(&fonts, ringkasan(&nilai))
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                .into(),
        );

        column(anak)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Start)
            .into()
    })
}

/// The summary sentence that changes along with it — proof that the values
/// really are owned by the application, not squirreled away inside the
/// control.
pub fn ringkasan(radio: &[bool]) -> String {
    match menyala(radio) {
        0 => "Semua radio mati.".to_string(),
        n => format!("{n} dari {} radio menyala.", radio.len()),
    }
}

/// The row of unusable switches, plus one with no visible label.
///
/// All three remain in the accessibility tree: a disabled control is
/// **announced** as dimmed, not hidden (§3.8).
fn mati(fonts: &Fonts, t: &Theme) -> View {
    row([
        View::from(switch(fonts, t, MATI).disabled(true)),
        View::from(switch(fonts, t, TERKUNCI).on(true).disabled(true)),
        View::from(switch_only(t).label(TANPA_LABEL).on(true)),
    ])
    .spacing(t.space(6.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessToggled};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(720.0, 640.0);
    const FRAME: Duration = Duration::from_micros(8_333);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// A node's rectangle **according to the accessibility tree** — the tests
    /// touch exactly where a screen reader announces.
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn keadaan(ui: &AppRuntime, label: &str) -> AccessToggled {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("{}", pohon.dump()))
            .node
            .toggled
            .unwrap_or_else(|| panic!("{label} tidak menyebut keadaannya"))
    }

    /// Pump frames until every spring stops (at most 2 simulated seconds).
    fn sampai_diam(ui: &mut AppRuntime) {
        let mut jam = Instant::now();
        for _ in 0..600 {
            ui.frame();
            jam += FRAME;
            if ui.animate_at(jam, silka_widgets::advance).is_empty() && ui.is_idle() {
                return;
            }
        }
        panic!("halaman tidak pernah berhenti bergerak");
    }

    /// One full tap at point `p`.
    fn ketuk(ui: &mut AppRuntime, p: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
    }

    #[test]
    fn ringkasan_ikut_data() {
        assert_eq!(menyala(&[true, false, true]), 2);
        assert_eq!(ringkasan(&[false, false, false]), "Semua radio mati.");
        assert_eq!(ringkasan(&[true, false, true]), "2 dari 3 radio menyala.");
    }

    #[test]
    fn halaman_menampilkan_semua_sakelar_dengan_peran_yang_benar() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let mut nama: Vec<&str> = vec![MODE_PESAWAT, MATI, TERKUNCI, TANPA_LABEL];
        nama.extend(RADIO);
        for label in nama {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::Switch, "{label}");
            assert!(
                e.node.toggled.is_some(),
                "{label} harus menyebut keadaannya"
            );
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }

        // Disabled ones are still announced, but promise no actions at all.
        let dimmed = pohon.find_label(MATI).unwrap();
        assert!(dimmed.node.disabled);
        assert!(dimmed.node.actions.is_empty());
        let hidup = pohon.find_label(RADIO[0]).unwrap();
        assert!(hidup.node.actions.contains(AccessActions::CLICK));
        assert!(hidup.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn ketukan_membalik_nilai_dan_ringkasannya_ikut() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[0]), AccessToggled::On);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::Off);

        let p = kotak(&ui, RADIO[1]).center();
        ketuk(&mut ui, p);
        assert!(!ui.is_idle(), "ketukan menjadwalkan tepat satu frame");
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::On);

        // The summary is built from the same data — if it follows, the values
        // really are owned by the application.
        let pohon = ui.access_tree();
        assert!(
            pohon.find_label(&ringkasan(&[true, true, true])).is_some(),
            "{}",
            pohon.dump()
        );
    }

    #[test]
    fn mode_pesawat_mematikan_semuanya_lalu_mengembalikannya() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::Off);

        let p = kotak(&ui, MODE_PESAWAT).center();
        ketuk(&mut ui, p);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::On);
        for label in RADIO {
            assert_eq!(keadaan(&ui, label), AccessToggled::Off, "{label}");
        }

        let p = kotak(&ui, MODE_PESAWAT).center();
        ketuk(&mut ui, p);
        sampai_diam(&mut ui);
        for label in RADIO {
            assert_eq!(keadaan(&ui, label), AccessToggled::On, "{label}");
        }
    }

    #[test]
    fn seretan_menyalakan_tanpa_satu_pun_klik() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Dark), &f);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::Off);

        let b = kotak(&ui, RADIO[1]);
        let y = b.center().y;
        let awal = Point::new(b.min_x() + 8.0, y);
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Down, awal, Duration::ZERO)
                .button(PointerButton::Primary),
        ));
        for i in 1..=4 {
            ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                Point::new(awal.x + 10.0 * i as f32, y),
                Duration::from_millis(8 * i),
            )));
        }
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(
                PointerPhase::Up,
                Point::new(awal.x + 40.0, y),
                Duration::from_millis(40),
            )
            .button(PointerButton::Primary),
        ));
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::On);
    }

    #[test]
    fn keyboard_bisa_mengubah_sakelar_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        sampai_diam(&mut ui);

        // Tab lands on the first switch; Space flips it.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::On);

        // The left arrow **sets** it off; pressing twice yields the same.
        for _ in 0..2 {
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowLeft),
                Duration::from_millis(40),
            )));
            sampai_diam(&mut ui);
        }
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::Off);
    }

    #[test]
    fn transisi_berjalan_beberapa_frame_lalu_aplikasi_kembali_idle() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        sampai_diam(&mut ui);

        let p = kotak(&ui, RADIO[1]).center();
        ketuk(&mut ui, p);
        ui.frame();

        // The thumb does not jump: it takes several animation frames to
        // arrive.
        let mut jam = Instant::now();
        let mut frame = 0;
        while frame < 600 {
            jam += FRAME;
            let dirty = ui.animate_at(jam, silka_widgets::advance);
            ui.frame();
            frame += 1;
            if dirty.is_empty() && ui.is_idle() {
                break;
            }
        }
        assert!(frame > 3, "transisinya melompat, cuma {frame} frame");
        assert!(ui.is_idle(), "spring yang sudah settle harus melepas GPU");
    }

    #[test]
    fn warna_selalu_datang_dari_token_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                sampai_diam(&mut ui);
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
                // An on track uses `accent`, an off one `separator`, the thumb
                // `on_accent` — no other color is born at the page layer.
                assert!(
                    latar.contains(&t.color.accent),
                    "{preset:?} {appearance:?}: tidak ada lintasan menyala"
                );
                assert!(latar.contains(&t.color.on_accent), "{preset:?}");
                assert!(latar.contains(&t.color.separator), "{preset:?}");
            }
        }
    }
}
