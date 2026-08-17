//! The input simulator, proved against a real widget rather than a toy view.
//!
//! Every assertion here is about a **seam**: a synthetic press has to travel
//! through the router, into a widget's gesture state, out through a callback,
//! into a signal, back through a rebuild, and land in the accessibility tree.
//! Testing that chain is the entire reason the harness exists (§9.5) — and
//! aiming by accessible name means these tests also keep the a11y contract
//! (§3.8) honest, because a button with no announced name cannot be clicked.

use std::cell::Cell;
use std::rc::Rc;

use silka_core::input::{NamedKey, PointerButton};
use silka_core::signals::{use_signal, Signal};
use silka_core::view::{column, View};
use silka_testing::{Case, Simulator};
use silka_theme::Theme;
use silka_widgets::{button_in, text_in, Fonts};

const NAIK: &str = "Tambah";
const TURUN: &str = "Kurang";

/// A counter page: two buttons and a line of text that states the value.
fn halaman(fonts: &Fonts, cx: &silka_core::app::BuildCtx) -> View {
    let theme: Theme = cx.expect_env::<Signal<Theme>>().get();
    let nilai = use_signal(|| 0i32);
    column([
        button_in(fonts, &theme, NAIK)
            .on_press(move || nilai.set(nilai.get() + 1))
            .into(),
        button_in(fonts, &theme, TURUN)
            .on_press(move || nilai.set(nilai.get() - 1))
            .into(),
        View::from(text_in(fonts, format!("Nilai {}", nilai.get()))),
    ])
    .spacing(8.0)
    .into()
}

fn sim() -> Simulator {
    let fonts = Fonts::bundled_only();
    let mut s = Simulator::case(Case::ALL[0], move |cx| halaman(&fonts, cx))
        .size(320.0, 240.0)
        .animator(silka_widgets::advance);
    s.settle();
    s
}

/// The value the page announces, read back through the a11y tree.
fn nilai(s: &Simulator) -> i32 {
    let tree = s.access_tree();
    let label = tree
        .entries()
        .iter()
        .filter_map(|e| e.node.label.clone())
        .find(|l| l.starts_with("Nilai "))
        .unwrap_or_else(|| panic!("tidak ada baris nilai:\n{}", tree.dump()));
    label
        .trim_start_matches("Nilai ")
        .parse()
        .expect("nilai berupa angka")
}

#[test]
fn klik_lewat_nama_aksesibilitas_menjalankan_callback() {
    let mut s = sim();
    assert_eq!(nilai(&s), 0);

    s.click_label(NAIK);
    s.settle();
    assert_eq!(nilai(&s), 1, "satu klik = satu kenaikan");

    s.click_label(TURUN);
    s.settle();
    assert_eq!(nilai(&s), 0);
}

#[test]
fn membatalkan_gestur_tidak_menghasilkan_klik() {
    // The distinction no coordinate-free test can make: a pointer that is
    // pressed and then taken away by the OS must not fire `on_press`.
    let mut s = sim();
    let titik = s.require_center(NAIK);
    s.move_to(titik);
    s.press();
    s.cancel();
    s.settle();
    assert_eq!(nilai(&s), 0, "cancel bukan klik");
}

#[test]
fn melepas_di_luar_tombol_tidak_menghasilkan_klik() {
    let mut s = sim();
    let titik = s.require_center(NAIK);
    s.move_to(titik);
    s.press();
    s.move_to(silka_paint::Point::new(titik.x, titik.y + 200.0));
    s.release();
    s.settle();
    assert_eq!(nilai(&s), 0, "lepas di luar batas bukan klik");
}

#[test]
fn tombol_bisa_dijalankan_dari_papan_ketik() {
    // Keyboard activation is an accessibility requirement, not a nicety: a
    // control reachable only by mouse is a control some users cannot use.
    let mut s = sim();
    s.tab();
    s.settle();
    s.key(NamedKey::Space);
    s.settle();
    assert_eq!(nilai(&s), 1, "Tab lalu Spasi harus menekan tombol pertama");
}

#[test]
fn klik_berulang_menumpuk() {
    let mut s = sim();
    for _ in 0..5 {
        s.click_label(NAIK);
        s.settle();
    }
    assert_eq!(nilai(&s), 5);
}

#[test]
fn tombol_sekunder_bukan_klik_utama() {
    let mut s = sim();
    let titik = s.require_center(NAIK);
    s.move_to(titik);
    s.press_button(PointerButton::Secondary);
    s.release_button(PointerButton::Secondary);
    s.settle();
    assert_eq!(nilai(&s), 0);
}

#[test]
fn aplikasi_kembali_diam_setelah_interaksi() {
    // The promise §3.5 makes: once a spring has arrived, nothing keeps asking
    // for frames. A harness that could not observe this would let a permanently
    // awake GPU ship.
    let mut s = sim();
    s.click_label(NAIK);
    let frames = s.settle();
    assert!(frames > 1, "animasi tekan harus butuh beberapa frame");
    assert!(s.ui().is_idle(), "setelah tenang tidak ada yang tertunda");
    assert!(!s.ui().is_animating());
}

#[test]
fn callback_hanya_dipanggil_sekali_per_klik() {
    // Counting in a `Cell` rather than through the signal: a double invocation
    // that happens to be idempotent in the UI is still a bug in the router.
    let hitung = Rc::new(Cell::new(0u32));
    let untuk_view = hitung.clone();
    let fonts = Fonts::bundled_only();
    let mut s = Simulator::case(Case::ALL[0], move |cx| {
        let theme: Theme = cx.expect_env::<Signal<Theme>>().get();
        let hitung = untuk_view.clone();
        column([button_in(&fonts, &theme, NAIK).on_press(move || {
            hitung.set(hitung.get() + 1);
        })])
        .into()
    })
    .size(320.0, 240.0)
    .animator(silka_widgets::advance);
    s.settle();

    s.click_label(NAIK);
    s.settle();
    assert_eq!(hitung.get(), 1);
}
