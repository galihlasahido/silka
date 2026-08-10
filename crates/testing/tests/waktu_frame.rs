//! The frame-time gate (REKOMENDASI §9.5).
//!
//! These tests measure the CPU side of a frame — rebuild → diff → layout →
//! paint — on trees big enough that a regression has somewhere to hide, and
//! fail when the 120 fps budget is blown. In a debug build the gate only
//! reports (see [`silka_testing::bench`]); CI runs it with `--release`.
//!
//! ## What these gates do and do not catch
//!
//! They come in two kinds, because neither kind is sufficient alone:
//!
//! - **Absolute budgets** state the product requirement — a full-page rebuild
//!   has to fit inside a frame. Today they pass with a wide margin, which is
//!   the point: they fire on a catastrophe (a pass that turned quadratic, a
//!   cache that stopped hitting), not on noise, and they hold on any machine.
//! - **Relative ratios** compare two workloads measured in the same process a
//!   moment apart. They cancel out the machine entirely, so they can be tight,
//!   and they are what actually catches an algorithmic regression.
//!
//! What neither catches is slow creep — three percent per pull request. That
//! needs a committed baseline measured on stable hardware, and a shared CI
//! runner is not stable hardware. Naming the gap is better than pretending a
//! wide budget closes it.

use std::time::Duration;

use silka_core::signals::Signal;
use silka_core::view::{column, fixed, row, View};
use silka_testing::bench::{Bench, Budget};
use silka_testing::{Case, Simulator};
use silka_theme::{RadiusToken, Theme};

/// A grid of decorated boxes: `rows * cols` painted nodes, each with a
/// background and a radius, which is what a dense business table looks like to
/// the layout and paint passes.
fn kisi(theme: &Theme, rows: usize, cols: usize) -> View {
    column((0..rows).map(|r| {
        row((0..cols).map(|c| {
            fixed(24.0, 18.0)
                .background(if (r + c) % 2 == 0 {
                    theme.color.surface
                } else {
                    theme.color.surface_elevated
                })
                .corners(theme.radius.corners(RadiusToken::Sm))
        }))
        .spacing(2.0)
    }))
    .spacing(2.0)
    .into()
}

fn sim(rows: usize, cols: usize) -> Simulator {
    let mut s = Simulator::case(Case::ALL[0], move |cx| {
        let theme: Theme = cx.expect_env::<Signal<Theme>>().get();
        kisi(&theme, rows, cols)
    })
    .size(1280.0, 800.0)
    .scale(2.0);
    s.settle();
    s
}

#[test]
fn frame_diam_tetap_di_dalam_anggaran_120fps() {
    // A frame with nothing dirty must be almost free — this is the number that
    // decides whether an idle window costs battery.
    let mut s = sim(24, 40);
    let samples = Bench::new("frame-diam").run_frames(&mut s);
    samples.assert_within(Budget::hz(120));
}

#[test]
fn membangun_ulang_pohon_penuh_tetap_di_dalam_anggaran() {
    // The opposite extreme: every frame writes a signal the root reads, so the
    // whole tree is rebuilt, diffed, laid out and painted. 60 Hz is the honest
    // budget for a full rebuild of a thousand nodes; 120 Hz is for frames that
    // rebuild a subtree.
    let mut s = sim(20, 30);
    let tema = s
        .ui()
        .env::<Signal<Theme>>()
        .expect("headless_app menitipkan tema");
    let awal = tema.get();
    let samples = Bench::new("bangun-ulang-penuh").iterations(60).run(|i| {
        // Writing the same value would be a no-op; alternate the appearance so
        // the rebuild is real.
        let mut t = awal;
        t.appearance = if i % 2 == 0 {
            silka_theme::Appearance::Light
        } else {
            silka_theme::Appearance::Dark
        };
        tema.set(t);
        s.frame();
    });
    samples.assert_within(Budget::hz(60));
}

#[test]
fn biaya_frame_tumbuh_masuk_akal_terhadap_jumlah_node() {
    // A relative check, which no absolute budget can replace: it holds on a
    // slow CI runner and on a fast laptop alike, and it catches the class of
    // regression that turns a linear pass quadratic.
    let mut kecil = sim(8, 10);
    let mut besar = sim(16, 20);
    let a = Bench::new("kisi-kecil")
        .iterations(40)
        .run_frames(&mut kecil);
    let b = Bench::new("kisi-besar")
        .iterations(40)
        .run_frames(&mut besar);
    eprintln!("{}\n{}", a.report(), b.report());

    // Four times the nodes may cost roughly four times the time; eight times
    // is the slack for cache effects and timer noise, and anything past it is
    // a pass that stopped being linear. The 20 µs floor keeps the ratio from
    // being computed against a measurement the clock cannot resolve.
    let batas = a.p50().max(Duration::from_micros(20)) * 8;
    assert!(
        b.p50() <= batas,
        "biaya per node meledak: {} vs {}",
        a.report(),
        b.report()
    );
}

#[test]
fn interaksi_tidak_membangunkan_seluruh_pohon() {
    // Per-component rebuild (§2.5) is a performance claim, and this is the
    // assertion that keeps it one: hovering may repaint, but it must not
    // rebuild a thousand components.
    let mut s = sim(20, 30);
    s.hover(silka_paint::Point::new(100.0, 100.0));
    let report = s.frame();
    assert!(
        report.rebuilt <= 1,
        "hover membangun ulang {} scope",
        report.rebuilt
    );
}
