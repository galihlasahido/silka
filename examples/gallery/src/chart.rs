//! Demo page: **charts** (`silka-chart`).
//!
//! One page holding every mark the library ships, because the interesting
//! questions are the ones that only appear when several charts share a window:
//! do they agree on their colors, do their tooltips fight each other, does one
//! of them keep the GPU awake after the others have settled.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Every mark | Line, area, grouped bars, stacked horizontal bars, and sparklines, all on one page |
//! | Spring data transitions | "New data" — the marks **travel** to their new values instead of jumping, carrying their velocity if you click again mid-flight |
//! | The tooltip rides the overlay system | Hover any chart: the panel is placed, flipped at the window edge, and sprung by `silka_widgets::overlay` — this page computes no positions |
//! | Colorblind-safe categorical palette | The series keep their hues across light/dark and both presets; slot order never shifts when a series is added |
//! | Locale-aware formatting | "Switch locale" — the same numbers become `1,5 jt` / `1.5M` / `1,5 Mio.` and the dates re-order with them |
//! | Time axis on real calendar boundaries | The line chart's labels sit on month starts, not every-30-days |
//! | Bars start at zero, lines do not | The bar axis includes 0; the line axis frames its own range |
//! | Empty state | "Clear" — the axes go, the message stays |
//! | AccessKit node | VoiceOver announces each chart and reads out what it shows |
//! | Both presets & dark mode | `--preset tailwind`, `--appearance dark` |
//! | Reduced motion | Turn on "Reduce motion" in the OS: values are immediately in place, the tooltip stops sliding |
//!
//! ```text
//! cargo run -p silka-gallery -- --page chart
//! cargo run -p silka-gallery -- --page chart --preset tailwind --appearance light
//! ```
//!
//! What is **absent** from this file: hand-assembled `Scene`s, layout
//! arithmetic, colour numbers, and any placement code for the tooltip.

use silka_chart::format::{Locale, NumberFormat};
use silka_chart::tooltip::{tooltip_overlay, ChartHover};
use silka_chart::{area_chart, bar_chart, line_chart, sparkline, ChartStyle, Date};
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{active_fonts, button, button_variant, overlay_layer, text, ButtonVariant};

/// The page title.
pub const JUDUL: &str = "Chart";

/// The line chart's name — also the anchor the a11y tests look for.
pub const NAMA_GARIS: &str = "Daily cash flow";
/// The area chart's name.
pub const NAMA_AREA: &str = "Cash balance";
/// The grouped bar chart's name.
pub const NAMA_BATANG: &str = "Revenue per quarter";
/// The stacked horizontal bar chart's name.
pub const NAMA_TUMPUK: &str = "Cost breakdown";

/// The button that regenerates the dataset.
pub const TOMBOL_DATA: &str = "New data";
/// The button that empties every chart.
pub const TOMBOL_KOSONG: &str = "Clear";
/// The button that fills them again.
pub const TOMBOL_ISI: &str = "Refill";
/// The button that cycles the locale.
pub const TOMBOL_LOCALE: &str = "Switch locale";

/// The empty-state text.
pub const KOSONG: &str = "No data for this period";

/// A chart's height, in spacing-scale steps (§2.6 — never a raw number).
const TINGGI_LANGKAH: f32 = 56.0;
/// One chart column's width, in spacing-scale steps.
const LEBAR_LANGKAH: f32 = 108.0;
/// How many days the line chart covers.
const HARI: usize = 120;

// ---------------------------------------------------------------------------
// Data that looks like data
// ---------------------------------------------------------------------------

/// One day of cash flow.
pub struct Hari {
    /// Day number since 1970-01-01 — the vocabulary `silka-chart` speaks for
    /// time (see `silka_chart::date`).
    pub tanggal: f64,
    /// Money in.
    pub masuk: f64,
    /// Money out.
    pub keluar: f64,
}

/// One quarter.
pub struct Kuartal {
    /// Its name, e.g. "K1".
    pub nama: String,
    /// Revenue.
    pub pendapatan: f64,
    /// Target.
    pub target: f64,
}

/// One cost category.
pub struct Biaya {
    /// The category name.
    pub nama: String,
    /// Fixed cost.
    pub tetap: f64,
    /// Variable cost.
    pub variabel: f64,
    /// One-off cost.
    pub sekali: f64,
}

/// A deterministic pseudo-random stream — the same `seed` always produces the
/// same data, so "New data" is reproducible and the tests below are not
/// flaky.
fn acak(seed: u64, i: u64) -> f64 {
    let mut x = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(i.wrapping_mul(1_442_695_040_888_963_407));
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x % 10_000) as f64 / 10_000.0
}

/// The daily series.
pub fn harian(seed: u64) -> Vec<Hari> {
    let mulai = Date::new(2026, 4, 1).to_days();
    (0..HARI)
        .map(|i| {
            let n = i as u64;
            let musim = ((i as f64) * 0.11).sin() * 0.35 + 1.0;
            Hari {
                tanggal: (mulai + i as i64) as f64,
                masuk: (6.0e6 + acak(seed, n) * 4.0e6) * musim,
                keluar: (4.2e6 + acak(seed, n + 977) * 2.6e6) * musim,
            }
        })
        .collect()
}

/// The running balance — a cumulative quantity, which is what earns an area
/// chart its fill.
pub fn saldo(seed: u64) -> Vec<Hari> {
    let mut total = 3.0e7;
    harian(seed)
        .into_iter()
        .map(|h| {
            total += h.masuk - h.keluar;
            Hari {
                tanggal: h.tanggal,
                masuk: total,
                keluar: 0.0,
            }
        })
        .collect()
}

/// The quarterly series.
pub fn kuartalan(seed: u64) -> Vec<Kuartal> {
    (0..4)
        .map(|i| Kuartal {
            nama: format!("K{}", i + 1),
            pendapatan: 8.0e8 + acak(seed, 31 + i) * 6.0e8,
            target: 1.0e9,
        })
        .collect()
}

/// The cost breakdown.
pub fn biaya(seed: u64) -> Vec<Biaya> {
    ["Operations", "Payroll", "Rent", "Marketing"]
        .iter()
        .enumerate()
        .map(|(i, nama)| {
            let n = i as u64;
            Biaya {
                nama: (*nama).to_string(),
                tetap: 1.5e8 + acak(seed, 61 + n) * 1.0e8,
                variabel: 0.8e8 + acak(seed, 91 + n) * 1.2e8,
                sekali: 0.2e8 + acak(seed, 121 + n) * 0.6e8,
            }
        })
        .collect()
}

/// The three locales the switcher cycles through.
pub const LOCALE: [Locale; 3] = [Locale::ID_ID, Locale::EN_US, Locale::DE_DE];

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    let seed = use_signal(|| 1u64);
    let terisi = use_signal(|| true);
    let idx_locale = use_signal(|| 0usize);
    // **One** hover signal for the whole page: the tooltip is a single overlay
    // entry, so two charts can never have one each open at the same time.
    let hover = use_signal(|| None::<ChartHover>);

    let l = LOCALE[idx_locale.get() % LOCALE.len()];
    let benih = if terisi.get() { seed.get() } else { 0 };
    let kosong = !terisi.get();

    let konten = column([
        judul(&t),
        kendali(&t, seed, terisi, idx_locale, l),
        row([
            garis(&t, l, benih, kosong, hover),
            area(&t, l, benih, kosong, hover),
        ])
        .spacing(t.space(6.0))
        .into(),
        row([
            batang(&t, l, benih, kosong, hover),
            tumpuk(&t, l, benih, kosong, hover),
        ])
        .spacing(t.space(6.0))
        .into(),
        percikan(&t, l, benih, kosong),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Start)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(6.0)));

    // Content first, the tooltip after: this order **is** the stacking order,
    // and the panel's position belongs entirely to the overlay system.
    overlay_layer(konten)
        .overlay(tooltip_overlay(
            &ChartStyle::from_theme(&t),
            hover.get().as_ref(),
            hover.get().map(|h| h.anchor()).unwrap_or_default(),
        ))
        .into()
}

fn judul(t: &Theme) -> View {
    column([
        View::from(
            text(JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                "Four kinds of mark on one page, all riding the same theme \
                 tokens and the same overlay system. Point anywhere inside the \
                 plot box — not exactly on the line — then press \"New data\" \
                 a few times to watch the values move while carrying their \
                 velocity.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH * 2.0)),
        ),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .into()
}

fn kendali(
    t: &Theme,
    seed: Signal<u64>,
    terisi: Signal<bool>,
    idx_locale: Signal<usize>,
    l: Locale,
) -> View {
    let isi = terisi.get();
    let label_isi = if isi { TOMBOL_KOSONG } else { TOMBOL_ISI };
    row([
        View::from(button(TOMBOL_DATA).on_press(move || seed.set(seed.get() + 1))),
        View::from(
            button_variant(label_isi, ButtonVariant::Secondary)
                .on_press(move || terisi.set(!terisi.get())),
        ),
        View::from(
            button_variant(TOMBOL_LOCALE, ButtonVariant::Ghost)
                .on_press(move || idx_locale.set(idx_locale.get() + 1)),
        ),
        View::from(
            text(l.tag)
                .size(t.typography.footnote.size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.accent)
                .single_line(),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

/// A chart's box: every chart on this page gets the same one, so the row
/// heights line up without a single hand-computed number.
fn kotak(t: &Theme, isi: impl Into<View>) -> View {
    constrained(
        BoxConstraints::new(
            t.space(LEBAR_LANGKAH),
            t.space(LEBAR_LANGKAH),
            t.space(TINGGI_LANGKAH),
            t.space(TINGGI_LANGKAH),
        ),
        isi,
    )
    .background(t.color.surface)
    .corners(t.corners_of(silka_theme::RadiusToken::Lg))
    .border(t.space_of(silka_theme::SpaceToken::Px), t.color.separator)
    .into()
}

/// The line chart: two series over a **time** axis.
fn garis(t: &Theme, l: Locale, seed: u64, kosong: bool, hover: Signal<Option<ChartHover>>) -> View {
    let data = if kosong { Vec::new() } else { harian(seed) };
    kotak(
        t,
        line_chart(data)
            .key("garis")
            .x(|d: &Hari| d.tanggal)
            .y_named("In", |d: &Hari| d.masuk)
            .y_named("Out", |d: &Hari| d.keluar)
            .time()
            .title(NAMA_GARIS)
            .legend(true)
            .animated(true)
            .locale(l)
            .value_format(NumberFormat::Compact)
            .empty(KOSONG)
            .on_hover(move |h| hover.set(h)),
    )
}

/// The area chart: **one** cumulative series, which is what earns a fill.
fn area(t: &Theme, l: Locale, seed: u64, kosong: bool, hover: Signal<Option<ChartHover>>) -> View {
    let data = if kosong { Vec::new() } else { saldo(seed) };
    kotak(
        t,
        area_chart(data)
            .key("area")
            .x(|d: &Hari| d.tanggal)
            .y_named("Balance", |d: &Hari| d.masuk)
            .time()
            .title(NAMA_AREA)
            .animated(true)
            .locale(l)
            .value_format(NumberFormat::Compact)
            .empty(KOSONG)
            .on_hover(move |h| hover.set(h)),
    )
}

/// Grouped vertical bars: revenue against target, side by side.
fn batang(
    t: &Theme,
    l: Locale,
    seed: u64,
    kosong: bool,
    hover: Signal<Option<ChartHover>>,
) -> View {
    let data = if kosong { Vec::new() } else { kuartalan(seed) };
    kotak(
        t,
        bar_chart(data)
            .key("batang")
            .x_label(|d: &Kuartal| d.nama.clone())
            .y_named("Revenue", |d: &Kuartal| d.pendapatan)
            .y_named("Target", |d: &Kuartal| d.target)
            .grouped()
            .title(NAMA_BATANG)
            .legend(true)
            .animated(true)
            .locale(l)
            .value_format(NumberFormat::Compact)
            .empty(KOSONG)
            .on_hover(move |h| hover.set(h)),
    )
}

/// Stacked **horizontal** bars — the layout to reach for when the category
/// names are words, because horizontal labels never need rotating.
fn tumpuk(
    t: &Theme,
    l: Locale,
    seed: u64,
    kosong: bool,
    hover: Signal<Option<ChartHover>>,
) -> View {
    let data = if kosong { Vec::new() } else { biaya(seed) };
    kotak(
        t,
        bar_chart(data)
            .key("tumpuk")
            .x_label(|d: &Biaya| d.nama.clone())
            .y_named("Fixed", |d: &Biaya| d.tetap)
            .y_named("Variable", |d: &Biaya| d.variabel)
            .y_named("One-off", |d: &Biaya| d.sekali)
            .stacked()
            .horizontal()
            .title(NAMA_TUMPUK)
            .legend(true)
            .animated(true)
            .locale(l)
            .value_format(NumberFormat::Compact)
            .empty(KOSONG)
            .on_hover(move |h| hover.set(h)),
    )
}

/// A row of sparklines: word-sized graphics beside their own numbers, which is
/// the only place a chart with no axes makes sense.
fn percikan(t: &Theme, l: Locale, seed: u64, kosong: bool) -> View {
    let hari = if kosong { Vec::new() } else { harian(seed) };
    let entri: Vec<View> = ["In", "Out", "Delta"]
        .iter()
        .enumerate()
        .map(|(i, nama)| {
            let nilai: Vec<f64> = hari
                .iter()
                .map(|h| match i {
                    0 => h.masuk,
                    1 => h.keluar,
                    _ => h.masuk - h.keluar,
                })
                .collect();
            let terakhir = nilai.last().copied().unwrap_or(0.0);
            row([
                View::from(
                    text(*nama)
                        .size(t.typography.footnote.size)
                        .color(t.color.secondary_label)
                        .single_line(),
                ),
                View::from(constrained(
                    BoxConstraints::new(t.space(24.0), t.space(24.0), t.space(6.0), t.space(6.0)),
                    sparkline(nilai)
                        .key(format!("percikan-{i}"))
                        .animated(true)
                        .empty(""),
                )),
                View::from(
                    text(NumberFormat::Compact.format(terakhir, &l))
                        .size(t.typography.footnote.size)
                        .weight(FontWeight::SEMIBOLD)
                        .color(t.color.label)
                        .single_line(),
                ),
            ])
            .spacing(t.space(2.0))
            .cross(CrossAlign::Center)
            .into()
        })
        .collect();

    row(entri)
        .spacing(t.space(6.0))
        .cross(CrossAlign::Center)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerEvent, PointerPhase};
    use silka_paint::{Command, Point, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(1024.0, 720.0);
    /// A Retina screen — which also exercises the scale-factor path.
    const SKALA: f64 = 2.0;

    fn aplikasi(preset: Preset, appearance: Appearance) -> AppRuntime {
        aplikasi_dengan(preset, appearance)
    }

    /// A headless app assembled **exactly the way `run_app` does it**.
    fn aplikasi_dengan(preset: Preset, appearance: Appearance) -> AppRuntime {
        let theme = Theme::new(preset, appearance);
        let ui = headless_app(theme, move |cx| halaman(cx)).sized(VIEWPORT.width, VIEWPORT.height);
        ui.env::<Signal<silka_core::app::ScaleFactor>>()
            .expect("run_app menitipkan scale factor")
            .set(silka_core::app::ScaleFactor(SKALA as f32));
        ui
    }

    /// A chart's box **according to the accessibility tree**.
    ///
    /// Deliberately via the a11y path: the geometry then comes from the layout
    /// result rather than from coordinates restated in the test, and what the
    /// test looks at is exactly what a screen reader points at (§3.8).
    fn kotak_chart(ui: &AppRuntime, nama: &str) -> silka_paint::Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(nama)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {nama:?}:\n{}", pohon.dump()))
            .bounds
    }

    #[test]
    fn halaman_terbangun_di_setiap_preset_dan_appearance() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let mut ui = aplikasi(preset, appearance);
                ui.frame();
                assert!(
                    !ui.scene().is_empty(),
                    "{preset:?}/{appearance:?}: halaman kosong"
                );
            }
        }
    }

    #[test]
    fn setiap_chart_punya_node_yang_bisa_dibacakan() {
        // Four charts, four descriptions — the promise §3.8 makes, checked on
        // the page rather than only in the crate's own tests.
        let mut ui = aplikasi(Preset::Cupertino, Appearance::Dark);
        ui.frame();
        let a11y = ui.tree().access_tree(None);
        for nama in [NAMA_GARIS, NAMA_AREA, NAMA_BATANG, NAMA_TUMPUK] {
            let e = a11y
                .find_label(nama)
                .unwrap_or_else(|| panic!("{nama} tidak ada di pohon a11y:\n{}", a11y.dump()));
            assert_eq!(e.node.role, silka_core::access::AccessRole::Image);
            assert!(
                e.node.value.as_ref().is_some_and(|v| !v.is_empty()),
                "{nama} harus punya ringkasan"
            );
        }
    }

    #[test]
    fn hover_membuka_tooltip_lewat_sistem_overlay() {
        let mut ui = aplikasi(Preset::Cupertino, Appearance::Dark);
        ui.frame();

        // Aim at the middle of the first chart's box. The exact point does not
        // matter — that is the whole point of hovering the plot rather than the
        // mark.
        let kotak = ui
            .tree()
            .access_tree(None)
            .find_label(NAMA_GARIS)
            .expect("chart garis")
            .bounds;
        let tengah = Point::new(
            kotak.origin.x + kotak.size.width * 0.5,
            kotak.origin.y + kotak.size.height * 0.5,
        );
        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            tengah,
            std::time::Duration::ZERO,
        )));
        ui.frame();

        let terlihat = silka_widgets::overlay::topmost(ui.tree());
        assert!(terlihat.is_some(), "tooltip harus terbuka lewat overlay");
    }

    #[test]
    fn chart_menggambar_batang_gridline_dan_label() {
        let mut ui = aplikasi(Preset::Tailwind, Appearance::Light);
        ui.frame();
        let quad = ui
            .scene()
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Quad(_)))
            .count();
        let glyph = ui
            .scene()
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::GlyphRun(_)))
            .count();
        assert!(
            quad > 40,
            "empat chart harus menghasilkan banyak quad: {quad}"
        );
        assert!(glyph > 20, "sumbu, legenda, dan judul: {glyph} glyph run");
    }

    #[test]
    fn nilai_bergerak_menuju_sasaran_lalu_berhenti() {
        // The claim `advance` makes: charts ask for frames while their springs
        // travel, and stop asking the moment they arrive. Driving it through
        // `AppRuntime::animate` is exactly the path `run_app_with` takes, so
        // this test exercises the wiring and not just the crate.
        let mut ui = aplikasi(Preset::Cupertino, Appearance::Light);
        ui.frame();
        assert!(silka_chart::is_animating(ui.tree()));

        // A caller-supplied clock, not `Instant::now()`: in a tight loop the
        // real clock advances by microseconds and the spring would never
        // arrive — the very trap §3.5 warns about from the other direction.
        let mut waktu = std::time::Instant::now();
        let mut frame = 0;
        loop {
            waktu += std::time::Duration::from_micros(8_333); // 120 Hz
            let dirty = ui.animate_at(waktu, silka_chart::advance);
            ui.frame();
            frame += 1;
            if !dirty.contains(silka_core::scheduler::Dirty::ANIMATION) || frame > 600 {
                break;
            }
        }
        assert!(frame <= 600, "spring chart tidak pernah selesai");
        assert!(!silka_chart::is_animating(ui.tree()));
    }

    #[test]
    fn keadaan_kosong_tetap_terbaca() {
        // Emptying the page must not blank it: the message is the state.
        let theme = Theme::cupertino(Appearance::Dark);
        let mut ui = headless_app(theme, move |_cx| {
            let hover = use_signal(|| None::<ChartHover>);
            overlay_layer(column([garis(
                &Theme::cupertino(Appearance::Dark),
                Locale::ID_ID,
                0,
                true,
                hover,
            )]))
            .into()
        })
        .sized(600.0, 400.0);
        ui.frame();
        assert!(ui
            .scene()
            .commands()
            .iter()
            .any(|c| matches!(c, Command::GlyphRun(_))));
    }

    // -- pixel proof: the marks are not merely in the scene, they are on screen

    /// Count the pixels inside a logical rect that are neither the page
    /// background nor the chart's own surface, and hash the region.
    ///
    /// Two colours rather than one because a chart sits on a card: the surface
    /// is not "content", so counting it would make an empty chart pass.
    fn cuplik(
        img: &silka_renderer::Rgba8Image,
        wilayah: silka_paint::Rect,
        latar: [silka_paint::Color; 2],
    ) -> (u32, u64) {
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
                let dekat = |warna: silka_paint::Color| {
                    let jauh = |c: u8, token: f32| (c as f32 - token * 255.0).abs() > 24.0;
                    !(jauh(p[0], warna.r) || jauh(p[1], warna.g) || jauh(p[2], warna.b))
                };
                if !dekat(latar[0]) && !dekat(latar[1]) {
                    n += 1;
                }
            }
        }
        (n, hash)
    }

    /// **The test this milestone would otherwise be missing.** Every other test
    /// on this page stops on the CPU side and would stay green with a blank
    /// window — the failure mode this project has already been bitten by three
    /// times (text not drawn in Phase 0; a render tree that could not paint and
    /// signals that triggered nothing in Phase 1).
    ///
    /// So: render the page through the **same** GPU path the window uses, and
    /// count pixels. Three claims, one of them negative:
    ///
    /// 1. The line chart's plot area really has marks in it.
    /// 2. A band of pure page padding really has none — which is what proves
    ///    the sampling threshold is not passing everything by accident.
    /// 3. Changing the dataset really changes those pixels.
    #[test]
    fn mark_chart_benar_benar_sampai_ke_piksel() {
        let Ok(gpu) = silka_renderer::Gpu::headless() else {
            eprintln!("dilewati: tidak ada GPU untuk render headless");
            return;
        };

        // The **ambient** engine, not a second one: the camera has to upload
        // the very atlas the layout measured against (§3.3).
        let fonts = active_fonts();
        let theme = Theme::cupertino(Appearance::Dark);
        let mut ui = aplikasi_dengan(Preset::Cupertino, Appearance::Dark);
        ui.frame();
        // Springs settled: what is measured is the finished chart, not a frame
        // caught halfway through its entrance.
        selesaikan(&mut ui);

        let mut target = silka_renderer::OffscreenTarget::new(
            &gpu,
            silka_renderer::SurfaceGeometry::from_logical(VIEWPORT, SKALA),
        )
        .expect("target headless");
        let gambar = |ui: &AppRuntime, target: &mut silka_renderer::OffscreenTarget| {
            fonts
                .with(|mesin| target.render_with_glyphs(&gpu, ui.scene(), mesin))
                .expect("render halaman chart")
        };

        let latar = [theme.color.background, theme.color.surface];
        let kotak = kotak_chart(&ui, NAMA_GARIS);

        let sebelum = gambar(&ui, &mut target);
        let (n0, h0) = cuplik(&sebelum, kotak, latar);
        assert!(
            n0 > 500,
            "chart garis nyaris tidak tergambar: hanya {n0} piksel bukan-latar"
        );

        // Negative control: the page's top padding, above the title, must be
        // exactly background.
        let kosong = silka_paint::Rect::new(0.0, 0.0, VIEWPORT.width, theme.space(4.0));
        assert_eq!(
            cuplik(&sebelum, kosong, latar).0,
            0,
            "ambang sampel salah: pita kosong sudah punya piksel bukan-latar"
        );

        // A new dataset must move pixels, not merely state.
        let tombol = kotak_chart(&ui, TOMBOL_DATA).center();
        klik(&mut ui, tombol);
        ui.frame();
        selesaikan(&mut ui);

        let sesudah = gambar(&ui, &mut target);
        let (n1, h1) = cuplik(&sesudah, kotak, latar);
        assert!(n1 > 500, "chart hilang setelah data berganti: {n1} piksel");
        assert_ne!(h0, h1, "data baru tidak mengubah satu piksel pun");
    }

    /// Put every chart spring at its destination, then draw the frame.
    ///
    /// `AppRuntime::animate` is the only door to a mutable tree, and it is also
    /// the door the real shell uses — so settling this way exercises the same
    /// wiring rather than reaching around it.
    fn selesaikan(ui: &mut AppRuntime) {
        ui.animate(|tree, _tick| {
            silka_chart::settle(tree);
            silka_core::scheduler::Dirty::PAINT
        });
        ui.frame();
    }

    /// One full click through the input layer: move, press, release.
    fn klik(ui: &mut AppRuntime, titik: Point) {
        use silka_core::input::PointerButton;
        let t = std::time::Duration::from_millis(10);
        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            titik,
            t,
        )));
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Down, titik, t).button(PointerButton::Primary),
        ));
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Up, titik, t).button(PointerButton::Primary),
        ));
    }

    /// A **visual review aid**, not a gate — hence `#[ignore]`.
    ///
    /// Renders the page in both presets through the real GPU path and drops the
    /// raw RGBA into the temp directory, so a human (or a golden-image job,
    /// §9.5) can look at what the pixel counters above only measure. Counting
    /// pixels proves something is drawn; it cannot tell you that an axis is
    /// speaking two units at once, which is exactly the kind of thing that
    /// looking finds in five seconds.
    ///
    /// ```text
    /// cargo test -p silka-gallery chart::tests::tulis_cuplikan -- --ignored
    /// ```
    #[test]
    #[ignore = "alat bantu tinjauan visual, bukan gerbang"]
    fn tulis_cuplikan_untuk_ditinjau() {
        let Ok(gpu) = silka_renderer::Gpu::headless() else {
            eprintln!("dilewati: tidak ada GPU untuk render headless");
            return;
        };
        for (nama, preset, appearance) in [
            ("cupertino-dark", Preset::Cupertino, Appearance::Dark),
            ("tailwind-light", Preset::Tailwind, Appearance::Light),
        ] {
            // Real system fonts here, unlike every other test on this page: the
            // point is to see what a user sees, not to be deterministic.
            let fonts = silka_widgets::Fonts::new();
            silka_widgets::install_fonts(&fonts);
            let mut ui = aplikasi_dengan(preset, appearance);
            ui.frame();
            selesaikan(&mut ui);
            let mut target = silka_renderer::OffscreenTarget::new(
                &gpu,
                silka_renderer::SurfaceGeometry::from_logical(VIEWPORT, SKALA),
            )
            .expect("target headless");
            let img = fonts
                .with(|m| target.render_with_glyphs(&gpu, ui.scene(), m))
                .expect("render halaman chart");
            // width, height, then raw RGBA — no PNG encoder is worth a
            // dependency for a debugging aid.
            let mut buf = Vec::with_capacity(img.pixels().len() + 8);
            buf.extend_from_slice(&img.width().to_le_bytes());
            buf.extend_from_slice(&img.height().to_le_bytes());
            buf.extend_from_slice(img.pixels());
            let path = std::env::temp_dir().join(format!("silka-chart-{nama}.rgba"));
            std::fs::write(&path, buf).expect("tulis cuplikan");
            eprintln!("cuplikan: {}", path.display());
        }
    }

    #[test]
    fn data_deterministik_agar_uji_tidak_goyah() {
        // "New data" has to be reproducible, otherwise every test on this page
        // becomes a coin flip.
        let a = harian(7);
        let b = harian(7);
        assert_eq!(a.len(), HARI);
        assert!(a.iter().zip(&b).all(|(x, y)| x.masuk == y.masuk));
        assert!(harian(8).iter().zip(&a).any(|(x, y)| x.masuk != y.masuk));
        let _ = Size::ZERO;
    }
}
