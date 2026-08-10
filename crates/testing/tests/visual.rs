//! Golden tests for the harness itself — and the worked example every other
//! crate copies (REKOMENDASI §9.5).
//!
//! The main scene is deliberately made of **shapes, not text**: squircle
//! corners, a double shadow and token colours are the parts of the design
//! system a renderer change can silently break, and they rasterise
//! near-identically on every driver. A second, smaller golden covers the glyph
//! path with [`Tolerance::TEXT`], because a harness that cannot capture text
//! cannot test a UI toolkit.
//!
//! Regenerate after an intentional visual change:
//!
//! ```bash
//! SILKA_GOLDEN=update cargo test -p silka-testing --test visual
//! ```

use silka_core::view::{column, fixed, View};
use silka_paint::Insets;
use silka_testing::{for_each_case, gpu_or_skip, Case, Simulator, Tolerance};
use silka_theme::{RadiusToken, ShadowToken, Theme};
use silka_widgets::{text, Fonts};

/// A card row: three boxes that exercise radius, shadow and the accent colour —
/// the three token families with a visual footprint.
fn kartu(theme: &Theme) -> View {
    column([
        fixed(160.0, 56.0)
            .background(theme.color.surface_elevated)
            .corners(theme.radius.corners(RadiusToken::Lg))
            .shadow(theme.shadow.get(ShadowToken::Md))
            .label("kartu"),
        fixed(160.0, 40.0)
            .background(theme.color.accent)
            .corners(theme.radius.corners(RadiusToken::Md))
            .label("aksi"),
        fixed(160.0, 24.0)
            .background(theme.color.surface_sunken)
            .corners(theme.radius.corners(RadiusToken::Sm))
            .border(1.0, theme.color.border)
            .label("alas"),
    ])
    .spacing(12.0)
    .padding(Insets::all(16.0))
    .into()
}

fn simulator(case: Case) -> Simulator {
    Simulator::case(case, move |cx| {
        let theme: Theme = cx.expect_env::<silka_core::signals::Signal<Theme>>().get();
        kartu(&theme)
    })
    .size(200.0, 160.0)
    .scale(2.0)
}

#[test]
fn kartu_terlihat_sama_di_setiap_preset_dan_appearance() {
    let mut gpu = gpu_or_skip!();
    for_each_case(|case| {
        let mut sim = simulator(case);
        sim.settle();
        let capture = sim.capture(&mut gpu);
        assert_eq!((capture.width(), capture.height()), (400, 320));
        case.golden("kartu")
            .tolerance(Tolerance::SHAPES)
            .assert(&capture);
    });
}

#[test]
fn preset_yang_berbeda_menghasilkan_gambar_yang_berbeda() {
    // The negative test the suite would be worthless without: if every case
    // rendered the same pixels, four green goldens would prove nothing at all.
    let mut gpu = gpu_or_skip!();
    let mut captures = Vec::new();
    for case in Case::ALL {
        let mut sim = simulator(case);
        sim.settle();
        captures.push((case.slug(), sim.capture(&mut gpu)));
    }
    for i in 0..captures.len() {
        for j in i + 1..captures.len() {
            let d = silka_testing::compare(&captures[i].1, &captures[j].1, Tolerance::SHAPES)
                .expect("ukuran sama");
            assert!(
                !d.is_match(),
                "{} dan {} menghasilkan gambar yang sama — token tidak sampai ke piksel",
                captures[i].0,
                captures[j].0
            );
        }
    }
}

#[test]
fn teks_ikut_tertangkap_lewat_sumber_glyph() {
    // The glyph path is separate plumbing: the atlas belongs to the app, not to
    // the harness, and a capture that forgot to pass it renders a page with
    // every word missing — a failure that looks like success to any test that
    // only counts commands. Bundled fonts only, so the rasteriser is the same
    // everywhere; system fonts would make the golden a picture of this laptop.
    let mut gpu = gpu_or_skip!();
    let fonts = Fonts::bundled_only();
    let untuk_view = fonts.clone();
    let case = Case::ALL[1]; // cupertino dark: light text on a dark ground
    let mut sim = Simulator::case(case, move |cx| {
        let theme: Theme = cx.expect_env::<silka_core::signals::Signal<Theme>>().get();
        column([
            text(&untuk_view, "Halo, silka")
                .size(28.0)
                .color(theme.color.label),
            text(&untuk_view, "Uji golden teks")
                .size(15.0)
                .color(theme.color.secondary_label),
        ])
        .spacing(8.0)
        .padding(Insets::all(16.0))
        .into()
    })
    .size(240.0, 100.0)
    .scale(2.0);
    fonts.set_scale_factor(2.0);
    sim.settle();

    let capture = fonts.with(|engine| sim.capture_with_glyphs(&mut gpu, engine));
    // The negative half of the assertion: without a glyph source the capture is
    // a flat background, so proving text is present at all comes first.
    let polos = sim.capture(&mut gpu);
    let d = silka_testing::compare(&polos, &capture, Tolerance::TEXT).expect("ukuran sama");
    assert!(
        !d.is_match(),
        "tangkapan dengan dan tanpa glyph tidak boleh sama — teks tidak tergambar"
    );

    case.golden("teks")
        .tolerance(Tolerance::TEXT)
        .assert(&capture);
}

#[test]
fn menggambar_ulang_adegan_yang_sama_menghasilkan_piksel_yang_sama() {
    // Determinism on one machine is the premise golden testing rests on.
    let mut gpu = gpu_or_skip!();
    let mut sim = simulator(Case::ALL[0]);
    sim.settle();
    let a = sim.capture(&mut gpu);
    let b = sim.capture(&mut gpu);
    assert_eq!(a, b, "render yang sama harus identik bit per bit");
}
