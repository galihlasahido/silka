//! Unit tests for the spring system: parameters, correctness of the
//! closed-form solution, convergence, retargeting, reduced motion, and the
//! seam with the scheduler's dirty flags.

use std::time::{Duration, Instant};

use silka_paint::{Color, Point, Size};

use crate::input::Velocity;
use crate::scheduler::Dirty;

use super::{
    Animatable, AnimationDriver, Motion, MotionRole, Propagator, Spring, SpringValue, Tolerance,
};

/// One frame on a 120 Hz display — the number comes from the display link, not
/// from a hard-coded 16.6 ms.
const FRAME: Duration = Duration::from_micros(8_333);

/// Run until settled; returns the number of frames.
fn jalankan(value: &mut SpringValue<f32>, motion: Motion) -> usize {
    let mut n = 0;
    while value.advance(FRAME, motion) {
        n += 1;
        assert!(n < 10_000, "spring tidak pernah berhenti");
    }
    n
}

/// RK4 numerical integration in f64 as an independent cross-check on the
/// closed-form solution. Deliberately not small steps of the same formula — if
/// the formula is wrong, this is the test that catches it.
fn integrasi(spring: Spring, x0: f64, v0: f64, t: f64) -> (f64, f64) {
    let w = spring.angular_frequency() as f64;
    let z = spring.damping_ratio() as f64;
    let a = |x: f64, v: f64| -w * w * x - 2.0 * z * w * v;
    let langkah = 20_000;
    let h = t / langkah as f64;
    let (mut x, mut v) = (x0, v0);
    for _ in 0..langkah {
        let (k1x, k1v) = (v, a(x, v));
        let (k2x, k2v) = (v + 0.5 * h * k1v, a(x + 0.5 * h * k1x, v + 0.5 * h * k1v));
        let (k3x, k3v) = (v + 0.5 * h * k2v, a(x + 0.5 * h * k2x, v + 0.5 * h * k2v));
        let (k4x, k4v) = (v + h * k3v, a(x + h * k3x, v + h * k3v));
        x += h / 6.0 * (k1x + 2.0 * k2x + 2.0 * k3x + k4x);
        v += h / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
    }
    (x, v)
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

#[test]
fn preset_mengikuti_wwdc23() {
    for (s, bounce) in [
        (Spring::smooth(), 0.0),
        (Spring::snappy(), 0.15),
        (Spring::bouncy(), 0.3),
    ] {
        assert!((s.duration() - 0.5).abs() < 1e-6, "{s:?}");
        assert!((s.bounce() - bounce).abs() < 1e-6, "{s:?}");
        // ζ = 1 − bounce, ω = 2π / duration.
        assert!((s.damping_ratio() - (1.0 - bounce)).abs() < 1e-6, "{s:?}");
        let w = core::f32::consts::TAU / 0.5;
        assert!((s.angular_frequency() - w).abs() < 1e-4, "{s:?}");
        assert!((s.stiffness() - w * w).abs() < 1e-2, "{s:?}");
    }
    assert!(!Spring::smooth().overshoots());
    assert!(Spring::snappy().overshoots());
    assert!(Spring::bouncy().overshoots());
}

#[test]
fn bounce_negatif_berarti_overdamped() {
    let s = Spring::new(0.5, -0.5);
    // ζ = 1 / (1 + bounce) = 2.
    assert!((s.damping_ratio() - 2.0).abs() < 1e-5, "{s:?}");
    assert!(!s.overshoots());
    // Dropping the bounce of a spring that never bounces is a no-op.
    assert_eq!(s.without_bounce(), s);
}

#[test]
fn parameter_fisik_bolak_balik() {
    for (m, k, c) in [(1.0, 200.0, 20.0), (2.0, 400.0, 80.0), (1.0, 100.0, 30.0)] {
        let s = Spring::physical(m, k, c);
        assert!((s.stiffness() - k / m).abs() < 1e-2, "{s:?}");
        assert!((s.damping() - c / m).abs() < 1e-2, "{s:?}");
        // A round trip through the perceptual representation leaves the
        // physics unchanged.
        let ulang = Spring::new(s.duration(), s.bounce());
        assert!((ulang.damping_ratio() - s.damping_ratio()).abs() < 1e-5);
    }
}

#[test]
fn parameter_gila_dijepit_bukan_memanik() {
    for s in [
        Spring::new(0.0, 0.0),
        Spring::new(-1.0, 0.0),
        Spring::new(f32::NAN, f32::NAN),
        Spring::new(0.5, 5.0),
        Spring::new(0.5, -5.0),
        Spring::physical(0.0, 0.0, -1.0),
    ] {
        assert!(s.duration() >= super::MIN_DURATION, "{s:?}");
        assert!(s.bounce().abs() <= super::MAX_BOUNCE, "{s:?}");
        assert!(
            s.damping_ratio().is_finite() && s.damping_ratio() > 0.0,
            "{s:?}"
        );
        let (x, v) = s.solve(1.0, 0.0, 0.01);
        assert!(x.is_finite() && v.is_finite(), "{s:?} -> {x} {v}");
    }
}

// ---------------------------------------------------------------------------
// Correctness of the closed-form solution
// ---------------------------------------------------------------------------

#[test]
fn propagator_nol_detik_adalah_identitas() {
    let s = Spring::bouncy();
    assert_eq!(s.propagator(0.0), Propagator::IDENTITY);
    assert_eq!(s.propagator(-1.0), Propagator::IDENTITY);
    assert_eq!(s.solve(3.0, -4.0, 0.0), (3.0, -4.0));
}

#[test]
fn closed_form_cocok_dengan_integrasi_numerik() {
    // All three regimes: underdamped, critically damped, overdamped.
    let springs = [
        Spring::bouncy(),
        Spring::smooth(),
        Spring::new(0.5, -0.5),
        Spring::new(0.25, 0.6),
    ];
    for s in springs {
        for (x0, v0) in [(1.0f64, 0.0f64), (0.0, 400.0), (-2.0, 150.0)] {
            for t in [0.01f64, 0.08, 0.3] {
                let (xr, vr) = integrasi(s, x0, v0, t);
                let (x, v) = s.solve(x0 as f32, v0 as f32, t as f32);
                assert!(
                    (x as f64 - xr).abs() < 1e-3 * (1.0 + xr.abs()),
                    "{s:?} t={t} x={x} rk4={xr}"
                );
                assert!(
                    (v as f64 - vr).abs() < 1e-2 * (1.0 + vr.abs()),
                    "{s:?} t={t} v={v} rk4={vr}"
                );
            }
        }
    }
}

#[test]
fn hasil_tidak_bergantung_ukuran_langkah() {
    // The signature property of the closed form: dropped frames do not shift
    // the animation.
    for s in [Spring::bouncy(), Spring::smooth(), Spring::new(0.4, -0.3)] {
        let (x_sekali, v_sekali) = s.solve(1.0, 200.0, 0.3);
        let (mut x, mut v) = (1.0f32, 200.0f32);
        for _ in 0..300 {
            let (nx, nv) = s.solve(x, v, 0.001);
            x = nx;
            v = nv;
        }
        assert!((x - x_sekali).abs() < 1e-3, "{s:?}: {x} vs {x_sekali}");
        assert!((v - v_sekali).abs() < 1e-2, "{s:?}: {v} vs {v_sekali}");
    }
}

// ---------------------------------------------------------------------------
// Convergence
// ---------------------------------------------------------------------------

#[test]
fn semua_preset_konvergen_tepat_ke_target() {
    for spring in [Spring::smooth(), Spring::snappy(), Spring::bouncy()] {
        let mut v = SpringValue::new(0.0).with_spring(spring);
        v.set_target(100.0);
        assert!(v.is_animating());
        let n = jalankan(&mut v, Motion::Full);
        assert!(!v.is_animating(), "{spring:?}");
        // Stopping means really stopping: no leftover 0.3 pt.
        assert_eq!(v.position(), 100.0, "{spring:?}");
        assert_eq!(v.velocity(), 0.0, "{spring:?}");
        // Perceptual duration of 0.5 s: settles in under two seconds at 120 Hz.
        assert!(n < 240, "{spring:?} butuh {n} frame");
        assert!(n > 30, "{spring:?} settle terlalu cepat ({n} frame)");
    }
}

#[test]
fn smooth_tidak_pernah_melewati_target() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::smooth());
    v.set_target(1.0);
    let mut sebelumnya = 0.0;
    while v.advance(FRAME, Motion::Full) {
        assert!(v.position() <= 1.0, "melewati target: {}", v.position());
        assert!(v.position() >= sebelumnya, "mundur: {}", v.position());
        sebelumnya = v.position();
    }
}

#[test]
fn bouncy_melewati_target_lalu_kembali() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::bouncy());
    v.set_target(1.0);
    let mut puncak = 0.0f32;
    while v.advance(FRAME, Motion::Full) {
        puncak = puncak.max(v.position());
    }
    assert!(puncak > 1.02, "bouncy harus memantul, puncak {puncak}");
    assert!(puncak < 1.30, "pantulan tidak boleh liar, puncak {puncak}");
    assert_eq!(v.position(), 1.0);
}

#[test]
fn overdamped_merayap_tanpa_melewati() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::new(0.5, -0.5));
    v.set_target(1.0);
    while v.advance(FRAME, Motion::Full) {
        assert!(v.position() <= 1.0);
    }
    assert_eq!(v.position(), 1.0);
}

#[test]
fn taksiran_waktu_settle_adalah_batas_atas_yang_ketat() {
    for spring in [
        Spring::smooth(),
        Spring::snappy(),
        Spring::bouncy(),
        Spring::new(0.5, -0.5),
    ] {
        let mut v = SpringValue::new(0.0).with_spring(spring);
        v.set_target(100.0);
        let taksiran = v.settling_duration(Motion::Full).as_secs_f32();
        let nyata = jalankan(&mut v, Motion::Full) as f32 * FRAME.as_secs_f32();
        // Upper bound: the simulation never takes longer than the estimate.
        assert!(
            nyata <= taksiran + FRAME.as_secs_f32(),
            "{spring:?}: nyata {nyata} melampaui taksiran {taksiran}"
        );
        // But still tight — not some arbitrarily inflated number.
        assert!(
            taksiran <= nyata * 1.5 + 0.05,
            "{spring:?}: taksiran {taksiran} terlalu longgar vs nyata {nyata}"
        );
    }
    // Something already at rest has no time left.
    assert_eq!(
        SpringValue::new(1.0f32).settling_duration(Motion::Full),
        Duration::ZERO
    );
}

#[test]
fn langkah_raksasa_langsung_mendarat() {
    // The closed form cannot blow up the way a numerical integrator would: a
    // `dt` of ten seconds simply means the value has arrived.
    let mut v = SpringValue::new(0.0).with_spring(Spring::bouncy());
    v.set_target(500.0);
    assert!(!v.advance(Duration::from_secs(10), Motion::Full));
    assert_eq!(v.position(), 500.0);
    assert!(!v.is_animating());
}

#[test]
fn dt_nol_tidak_menggerakkan_tapi_tetap_minta_frame() {
    let mut v = SpringValue::new(0.0);
    v.set_target(10.0);
    assert!(v.advance(Duration::ZERO, Motion::Full));
    assert_eq!(v.position(), 0.0);
    assert!(v.is_animating());
}

#[test]
fn toleransi_longgar_berhenti_lebih_cepat() {
    let mut halus = SpringValue::new(0.0).with_spring(Spring::snappy());
    let mut kasar = SpringValue::new(0.0)
        .with_spring(Spring::snappy())
        .with_tolerance(Tolerance::new(0.5, 5.0));
    halus.set_target(100.0);
    kasar.set_target(100.0);
    assert!(jalankan(&mut kasar, Motion::Full) < jalankan(&mut halus, Motion::Full));
    // Whatever the tolerance, the final value is still snapped to the target.
    assert_eq!(kasar.position(), 100.0);
}

#[test]
fn toleransi_menuntut_diam_sekaligus_dekat() {
    let t = Tolerance::POINTS;
    assert!(t.settled(0.0, 0.0));
    assert!(
        !t.settled(0.0, 10.0),
        "melintas target dengan kecepatan penuh"
    );
    assert!(!t.settled(10.0, 0.0), "berhenti jauh dari target");
    assert_eq!(Tolerance::new(-1.0, -2.0), Tolerance::new(1.0, 2.0));
}

// ---------------------------------------------------------------------------
// Retarget & handoff
// ---------------------------------------------------------------------------

#[test]
fn retarget_membawa_velocity_tanpa_patahan() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::snappy());
    v.set_target(100.0);
    for _ in 0..10 {
        v.advance(FRAME, Motion::Full);
    }
    let (p, kecepatan) = (v.position(), v.velocity());
    assert!(kecepatan > 1.0, "harus sedang bergerak: {kecepatan}");

    // The heart of WWDC23: retargeting cancels nothing — this frame's position
    // and velocity are still exactly the same afterwards.
    v.set_target(-50.0);
    assert_eq!(v.position(), p);
    assert_eq!(v.velocity(), kecepatan);
    assert_eq!(v.target(), -50.0);
    assert!(v.is_animating());

    jalankan(&mut v, Motion::Full);
    assert_eq!(v.position(), -50.0);
}

#[test]
fn retarget_ke_posisi_sekarang_tidak_membekukan_gerakan() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::snappy());
    v.set_target(100.0);
    for _ in 0..12 {
        v.advance(FRAME, Motion::Full);
    }
    let p = v.position();
    // The target is moved exactly onto the current position: momentum must
    // still carry it past and then back. If velocity were dropped, the value
    // would stop dead — a seam the eye can see.
    v.set_target(p);
    assert!(v.is_animating());
    assert!(v.advance(FRAME, Motion::Full));
    assert!(v.position() > p, "momentum hilang: {} vs {p}", v.position());

    jalankan(&mut v, Motion::Full);
    assert_eq!(v.position(), p);
}

#[test]
fn retarget_setiap_frame_tetap_konvergen() {
    // The dragging pattern: the target follows the finger every frame, then
    // the finger stops.
    let mut v = SpringValue::new(0.0).with_spring(Spring::smooth());
    let mut jari = 0.0f32;
    for _ in 0..60 {
        jari += 3.0;
        v.set_target(jari);
        assert!(v.advance(FRAME, Motion::Full));
        assert!(v.position().is_finite());
        assert!(v.position() <= jari + 1e-3, "menyalip jari");
    }
    jalankan(&mut v, Motion::Full);
    assert_eq!(v.position(), jari);
}

#[test]
fn retarget_ke_target_yang_sama_tidak_mengubah_lintasan() {
    let mut a = SpringValue::new(0.0).with_spring(Spring::bouncy());
    let mut b = a;
    a.set_target(50.0);
    b.set_target(50.0);
    for _ in 0..40 {
        a.advance(FRAME, Motion::Full);
        b.set_target(50.0); // idempotent
        b.advance(FRAME, Motion::Full);
        assert_eq!(a.position(), b.position());
        assert_eq!(a.velocity(), b.velocity());
    }
}

#[test]
fn handoff_fling_menjadi_spring() {
    // The velocity tracker hands over the finger's speed at release; the
    // spring carries it on from a value that was resting on its target.
    let mut v = SpringValue::new(0.0).with_spring(Spring::smooth());
    assert!(!v.is_animating());
    v.set_velocity(400.0);
    assert!(v.is_animating(), "fling harus membangunkan spring");

    let mut puncak = 0.0f32;
    while v.advance(FRAME, Motion::Full) {
        puncak = puncak.max(v.position());
    }
    assert!(puncak > 5.0, "fling tidak menghasilkan gerakan: {puncak}");
    assert_eq!(v.position(), 0.0);
}

#[test]
fn dorongan_velocity_bertumpuk() {
    let mut v = SpringValue::new(0.0);
    v.set_velocity(100.0);
    v.add_velocity(50.0);
    assert_eq!(v.velocity(), 150.0);
    // Nonsensical values are ignored rather than spreading as NaN.
    v.set_velocity(f32::NAN);
    assert_eq!(v.velocity(), 150.0);
    v.set_target(f32::INFINITY);
    assert_eq!(v.target(), 0.0);
}

#[test]
fn jump_to_menghentikan_semuanya() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::bouncy());
    v.set_target(100.0);
    for _ in 0..10 {
        v.advance(FRAME, Motion::Full);
    }
    v.jump_to(7.0);
    assert_eq!(v.position(), 7.0);
    assert_eq!(v.target(), 7.0);
    assert_eq!(v.velocity(), 0.0);
    assert!(!v.is_animating());
    assert!(!v.advance(FRAME, Motion::Full));
}

#[test]
fn ganti_spring_di_tengah_gerakan_tidak_mengguncang_keadaan() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::bouncy());
    v.set_target(100.0);
    for _ in 0..8 {
        v.advance(FRAME, Motion::Full);
    }
    let (p, kecepatan) = (v.position(), v.velocity());
    v.set_spring(Spring::smooth());
    assert_eq!(v.position(), p);
    assert_eq!(v.velocity(), kecepatan);
    assert_eq!(v.spring(), Spring::smooth());
    jalankan(&mut v, Motion::Full);
    assert_eq!(v.position(), 100.0);
}

#[test]
fn ganti_peran_di_tengah_gerakan_tidak_mengguncang_keadaan() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::smooth());
    v.set_target(100.0);
    for _ in 0..8 {
        v.advance(FRAME, Motion::Full);
    }
    let (p, kecepatan) = (v.position(), v.velocity());

    // The `&mut` counterpart of `decorative()`, used on a view's `update` path.
    assert_eq!(v.role(), MotionRole::Essential);
    v.set_role(MotionRole::Decorative);
    assert_eq!(v.role(), MotionRole::Decorative);
    assert_eq!(v.position(), p, "posisi tidak boleh melompat");
    assert_eq!(v.velocity(), kecepatan, "velocity harus terbawa");

    // The new role takes effect immediately: reduced motion eats what is left
    // of the movement.
    assert!(!v.advance(FRAME, Motion::Reduced));
    assert_eq!(v.position(), 100.0);

    // And it can be put back.
    v.set_role(MotionRole::Essential);
    assert_eq!(v.role(), MotionRole::Essential);
}

// ---------------------------------------------------------------------------
// Reduced motion
// ---------------------------------------------------------------------------

#[test]
fn reduced_motion_membuang_pantulan_bukan_gerakannya() {
    let mut v = SpringValue::new(0.0).with_spring(Spring::bouncy());
    v.set_target(1.0);
    let mut frame = 0;
    while v.advance(FRAME, Motion::Reduced) {
        frame += 1;
        assert!(v.position() <= 1.0, "masih memantul: {}", v.position());
    }
    assert!(frame > 10, "gerakan yang menjelaskan tidak boleh dihapus");
    assert_eq!(v.position(), 1.0);
    assert_eq!(Motion::Reduced.spring(Spring::bouncy()).bounce(), 0.0);
    assert_eq!(Motion::Full.spring(Spring::bouncy()), Spring::bouncy());
}

#[test]
fn reduced_motion_mematikan_gerakan_dekoratif() {
    let mut v = SpringValue::new(0.0)
        .with_spring(Spring::bouncy())
        .decorative();
    assert_eq!(v.role(), MotionRole::Decorative);
    v.set_target(1.0);
    assert!(!v.advance(FRAME, Motion::Reduced), "harus selesai seketika");
    assert_eq!(v.position(), 1.0);
    assert_eq!(v.velocity(), 0.0);
    assert_eq!(v.settling_duration(Motion::Reduced), Duration::ZERO);

    // Under full motion the same decorative value still animates.
    let mut w = SpringValue::new(0.0)
        .with_spring(Spring::bouncy())
        .decorative();
    w.set_target(1.0);
    assert!(w.advance(FRAME, Motion::Full));
}

#[test]
fn motion_dari_flag_platform() {
    assert_eq!(Motion::from_reduced(true), Motion::Reduced);
    assert_eq!(Motion::from_reduced(false), Motion::Full);
    assert!(Motion::Reduced.is_reduced());
    assert_eq!(Motion::default(), Motion::Full);
    assert_eq!(MotionRole::default(), MotionRole::Essential);
    assert!(!Motion::Reduced.suppresses(MotionRole::Essential));
    assert!(Motion::Reduced.suppresses(MotionRole::Decorative));
    assert!(!Motion::Full.suppresses(MotionRole::Decorative));
    assert_eq!(Motion::Reduced.label(), "reduced");
}

// ---------------------------------------------------------------------------
// Vector values
// ---------------------------------------------------------------------------

#[test]
fn spring_bekerja_untuk_point_size_dan_color() {
    let mut p = SpringValue::new(Point::ZERO);
    p.set_target(Point::new(100.0, 50.0));
    while p.advance(FRAME, Motion::Full) {}
    assert_eq!(p.position(), Point::new(100.0, 50.0));

    let mut s = SpringValue::new(Size::ZERO).with_spring(Spring::snappy());
    s.set_target(Size::new(320.0, 200.0));
    while s.advance(FRAME, Motion::Full) {}
    assert_eq!(s.position(), Size::new(320.0, 200.0));

    let mut c = SpringValue::new(Color::hex(0x1C1C1E));
    assert_eq!(c.tolerance(), Tolerance::COLOR);
    c.set_target(Color::hex(0x0A84FF));
    while c.advance(FRAME, Motion::Full) {}
    assert_eq!(c.position(), Color::hex(0x0A84FF));
}

#[test]
fn sumbu_vektor_tidak_pernah_keluar_sinkron() {
    // One propagator for every component: a diagonal path stays straight.
    let mut p = SpringValue::new(Point::ZERO).with_spring(Spring::bouncy());
    p.set_target(Point::new(100.0, 50.0));
    while p.advance(FRAME, Motion::Full) {
        let q = p.position();
        assert!((q.y * 2.0 - q.x).abs() < 1e-2, "lintasan bengkok: {q:?}");
    }
}

// ---------------------------------------------------------------------------
// Seam with the scheduler
// ---------------------------------------------------------------------------

#[test]
fn driver_meminta_frame_hanya_selama_ada_yang_bergerak() {
    let mut d = AnimationDriver::new();
    let mut v = SpringValue::new(0.0).with_spring(Spring::snappy());
    let mut now = Instant::now();

    // Nothing is moving: no frame is requested.
    let tick = d.begin_frame(now);
    let _ = tick.advance(&mut v);
    assert_eq!(d.end_frame(tick), Dirty::NONE);
    assert!(!d.is_animating());

    v.set_target(100.0);
    let mut frame = 0;
    loop {
        let tick = d.begin_frame(now);
        let posisi = tick.advance(&mut v);
        assert!(posisi.is_finite());
        let dirty = d.end_frame(tick);
        now += FRAME;
        frame += 1;
        if dirty == Dirty::NONE {
            break;
        }
        assert_eq!(dirty, Dirty::ANIMATION);
        assert!(frame < 1_000);
    }
    assert!(!d.is_animating());
    assert_eq!(v.position(), 100.0);
    assert!(frame > 10);
}

#[test]
fn driver_melupakan_jam_setelah_idle() {
    let mut d = AnimationDriver::new();
    let mut v = SpringValue::new(0.0);
    let now = Instant::now();

    v.set_target(1.0);
    let tick = d.begin_frame(now);
    tick.advance(&mut v);
    assert_eq!(d.end_frame(tick), Dirty::ANIMATION);

    // The second frame is one frame away: an honest dt.
    let tick = d.begin_frame(now + FRAME);
    assert_eq!(tick.dt(), FRAME);
    tick.advance(&mut v);
    d.end_frame(tick);

    // Finish the animation, then sit idle for a long time.
    loop {
        let tick = d.begin_frame(now + Duration::from_secs(1));
        tick.advance(&mut v);
        if d.end_frame(tick) == Dirty::NONE {
            break;
        }
    }
    // A new animation after five idle seconds starts from dt zero, not five
    // seconds.
    let tick = d.begin_frame(now + Duration::from_secs(6));
    assert_eq!(tick.dt(), Duration::ZERO);
    d.end_frame(tick);
}

#[test]
fn driver_meminta_frame_saat_preferensi_gerakan_berubah() {
    let mut d = AnimationDriver::new();
    assert_eq!(d.motion(), Motion::Full);
    assert_eq!(d.set_motion(Motion::Full), Dirty::NONE);
    assert_eq!(d.set_motion(Motion::Reduced), Dirty::ANIMATION);
    assert_eq!(d.motion(), Motion::Reduced);
    // The tick inherits the preference in effect.
    let tick = d.begin_frame(Instant::now());
    assert_eq!(tick.motion(), Motion::Reduced);
    assert!(!tick.is_active());
}

#[test]
fn sumber_gerakan_lain_boleh_menahan_frame_tetap_hidup() {
    let mut d = AnimationDriver::new();
    let tick = d.begin_frame(Instant::now());
    tick.keep_awake();
    assert!(tick.is_active());
    assert_eq!(d.end_frame(tick), Dirty::ANIMATION);
}

#[test]
fn reset_membuang_jam() {
    let mut d = AnimationDriver::new();
    let now = Instant::now();
    let tick = d.begin_frame(now);
    tick.keep_awake();
    d.end_frame(tick);
    d.reset();
    let tick = d.begin_frame(now + Duration::from_secs(2));
    assert_eq!(tick.dt(), Duration::ZERO);
}

// ---------------------------------------------------------------------------
// Gesture handoff -> spring
// ---------------------------------------------------------------------------

#[test]
fn velocity_gesture_jadi_velocity_spring_tanpa_tertukar_sumbu() {
    // The axes must not be swapped: x stays x, y stays y (positive = down).
    let p = Point::from(Velocity::new(-120.0, 900.0));
    assert_eq!(p, Point::new(-120.0, 900.0));
}

#[test]
fn handoff_fling_meneruskan_gerakan_tanpa_patahan() {
    let mut v = SpringValue::new(Point::new(0.0, 0.0)).with_spring(Spring::smooth());
    v.set_target(Point::new(0.0, -300.0));

    // A few frames first: the gesture is handed over *mid*-motion.
    v.advance(FRAME, Motion::Full);
    let sebelum = v.position();

    v.hand_off(Velocity::new(0.0, -2000.0));
    assert_eq!(v.velocity(), Point::new(0.0, -2000.0));
    // The position does not jump — only the velocity is replaced.
    assert_eq!(v.position(), sebelum);
    assert!(v.is_animating());

    let mut n = 0;
    while v.advance(FRAME, Motion::Full) {
        n += 1;
        assert!(n < 100_000, "handoff harus tetap konvergen");
    }
    assert_eq!(v.position(), Point::new(0.0, -300.0));
    assert_eq!(v.velocity(), Point::new(0.0, 0.0));
}

#[test]
fn handoff_saat_diam_di_target_membangunkan_animasi() {
    let mut v = SpringValue::new(Point::new(0.0, 0.0));
    assert!(!v.is_animating());
    v.hand_off(Velocity::new(0.0, -1500.0));
    assert!(
        v.is_animating(),
        "dorongan dari gesture harus membangunkan spring walau target sama"
    );
    while v.advance(FRAME, Motion::Full) {}
    assert_eq!(v.position(), Point::new(0.0, 0.0));
}

#[test]
fn handoff_velocity_nol_tidak_membangunkan_apa_pun() {
    let mut v = SpringValue::new(Point::new(0.0, 0.0));
    v.hand_off(Velocity::ZERO);
    assert!(!v.is_animating());
}

#[test]
fn handoff_dibatasi_besarannya_sebelum_diserahkan() {
    let liar = Velocity::new(0.0, -90_000.0).clamp_magnitude(4_000.0);
    let mut v = SpringValue::new(Point::new(0.0, 0.0));
    v.hand_off(liar);
    assert!(v.velocity().magnitude() <= 4_000.0 + 1e-3);
}

/// Does a 0 → 1 spring ever stop, at the tick rate a 120 Hz display produces?
///
/// Diagnostic for an application that kept redrawing forever once a text field
/// took focus.
#[test]
#[ignore = "diagnostic, not a gate"]
fn diagnosis_spring_fokus_berhenti_atau_tidak() {
    for (nama, spring) in [
        ("snappy", Spring::snappy()),
        ("smooth", Spring::smooth()),
        ("bouncy", Spring::bouncy()),
    ] {
        let mut v = SpringValue::new(0.0f32).with_spring(spring);
        v.set_target(1.0);
        let dt = Duration::from_secs_f64(1.0 / 120.0);
        let mut n = 0;
        while v.is_animating() && n < 100_000 {
            v.advance(dt, Motion::Full);
            n += 1;
        }
        eprintln!(
            "  {nama:8} -> {n} tick ({:.2} detik) · posisi {:.6} · masih animasi: {}",
            n as f64 / 120.0,
            v.position(),
            v.is_animating()
        );
    }
}
