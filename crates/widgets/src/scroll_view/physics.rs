//! Scroll physics: **macOS-style rubber banding**, scrollbar geometry, and
//! keyboard steps — all pure functions.
//!
//! Deliberately kept apart from the node. What lives here is the one part of
//! `scroll_view` that can go wrong *quietly*: a bounce that snaps back too
//! hard, a thumb that sits half a pixel off the finger dragging it, or a scroll
//! that sticks one point short of the end. As pure functions they can be tested
//! without a tree, without a GPU, and without a clock (REKOMENDASI §9.5).
//!
//! ## Rubber band
//!
//! The formula Apple uses in UIScrollView:
//!
//! ```text
//! f(x) = L · x / (x + L)        L = coefficient × viewport size
//! ```
//!
//! `x` is the raw pull distance and `f(x)` the visible displacement: it starts
//! out 1:1 under the finger, then grows heavier, and never passes `L`. Its
//! derivative — [`rubber_band_factor`] — is the form actually used while
//! scrolling, because scroll events arrive as **deltas**, not as a total
//! distance from the point where the pull began:
//!
//! ```text
//! f'(x) = (1 − y/L)²           y = the displacement currently visible
//! ```
//!
//! The two are mathematically consistent, and a test proves it by integrating
//! that factor back into `f`.

/// macOS/UIKit rubber-band coefficient: maximum displacement = 0.55 × viewport.
pub const RUBBER_BAND: f32 = 0.55;

/// Length-comparison threshold in logical points.
///
/// Far below a single physical pixel on a 3× display, so "equal" here means
/// equal to the eye while staying stable against f32 rounding error.
const EPS: f32 = 1.0 / 1024.0;

// ---------------------------------------------------------------------------
// Scroll bounds
// ---------------------------------------------------------------------------

/// The largest scroll offset that still leaves content on screen.
///
/// Zero means the content fits entirely — and that is more than a number: a
/// container whose `max_scroll` is zero **must not** swallow scroll events, so
/// that the container above it takes over instead (scroll chaining).
pub fn max_scroll(viewport: f32, content: f32) -> f32 {
    (content - viewport).max(0.0)
}

/// Clamp a scroll position into the valid range.
pub fn clamp_scroll(offset: f32, max: f32) -> f32 {
    offset.clamp(0.0, max.max(0.0))
}

/// Displacement past the bounds: negative at the top/left, positive at the
/// bottom/right, zero while inside.
pub fn overshoot(offset: f32, max: f32) -> f32 {
    if offset < 0.0 {
        offset
    } else if offset > max {
        offset - max
    } else {
        0.0
    }
}

/// The nearest edge for a position — where the rubber band bounces back to.
pub fn nearest_bound(offset: f32, max: f32) -> f32 {
    clamp_scroll(offset, max)
}

// ---------------------------------------------------------------------------
// Rubber band
// ---------------------------------------------------------------------------

/// Maximum displacement allowed past an edge (`L` in the formula above).
pub fn overscroll_limit(viewport: f32, coefficient: f32) -> f32 {
    (viewport * coefficient).max(0.0)
}

/// Damping factor for a scroll **delta** once the content is past an edge.
///
/// 1 exactly at the edge (free movement), 0 at maximum displacement (nothing
/// left to pull). This is `f'` from Apple's formula, expressed in terms of the
/// displacement currently visible, so nothing has to remember the raw pull
/// distance.
pub fn rubber_band_factor(overshoot: f32, viewport: f32, coefficient: f32) -> f32 {
    let limit = overscroll_limit(viewport, coefficient);
    if limit <= 0.0 {
        return 0.0;
    }
    let y = (overshoot.abs() / limit).clamp(0.0, 1.0);
    (1.0 - y) * (1.0 - y)
}

/// Closed form of Apple's formula: raw pull distance → visible displacement.
///
/// The sign follows the sign of `raw`.
pub fn rubber_band_offset(raw: f32, viewport: f32, coefficient: f32) -> f32 {
    let limit = overscroll_limit(viewport, coefficient);
    if limit <= 0.0 {
        return 0.0;
    }
    let x = raw.abs();
    (limit * x / (x + limit)).copysign(raw)
}

/// Inverse of [`rubber_band_offset`]: visible displacement → the raw pull
/// distance that produced it.
///
/// This is what makes the rubber band **step-size independent**. Scroll events
/// arrive as deltas tens of points wide, not as infinitesimal movement;
/// multiplying a delta that large by [`rubber_band_factor`] drifts well off the
/// curve (and right at the edge, where the factor is still 1, damps nothing at
/// all). So what [`apply_delta`] does instead is go back to the raw pull
/// distance, add the delta there, and map it forward again — the result is
/// exactly what dragging a finger that far would give, however coarse the
/// sampling.
pub fn rubber_band_raw(offset: f32, viewport: f32, coefficient: f32) -> f32 {
    let limit = overscroll_limit(viewport, coefficient);
    let y = offset.abs();
    if limit <= 0.0 || y >= limit {
        return f32::INFINITY.copysign(offset);
    }
    (limit * y / (limit - y)).copysign(offset)
}

/// Apply one scroll delta, with rubber banding past the edges.
///
/// Three rules that have to hold together, and the order matters:
///
/// 1. The part of the movement that travels **back** into bounds is never
///    damped — stretched content must follow the finger 1:1 on the way home.
/// 2. The part that lands **inside** the bounds moves 1:1.
/// 3. Whatever is left, out past the edge, is damped by [`rubber_band_factor`]
///    and never passes [`overscroll_limit`].
pub fn apply_delta(current: f32, delta: f32, max: f32, viewport: f32, coefficient: f32) -> f32 {
    if !delta.is_finite() || delta == 0.0 {
        return current;
    }
    let max = max.max(0.0);
    let mut pos = current;
    let mut sisa = delta;

    // 1. Head home first: any existing displacement is spent without damping.
    if pos < 0.0 && sisa > 0.0 {
        let langkah = sisa.min(-pos);
        pos += langkah;
        sisa -= langkah;
    } else if pos > max && sisa < 0.0 {
        let langkah = (-sisa).min(pos - max);
        pos -= langkah;
        sisa += langkah;
    }
    if sisa == 0.0 {
        return pos;
    }

    // 2. Inside the bounds: 1:1.
    if pos >= 0.0 && pos <= max {
        let ruang = if sisa > 0.0 { max - pos } else { pos };
        let langkah = sisa.abs().min(ruang) * sisa.signum();
        pos += langkah;
        sisa -= langkah;
    }
    if sisa == 0.0 {
        return pos;
    }

    // 3. Past the edge: along the curve, not by a per-step multiplication.
    let (batas, arah) = if sisa > 0.0 {
        (max, 1.0f32)
    } else {
        (0.0, -1.0)
    };
    let simpangan = (pos - batas).abs();
    let mentah = rubber_band_raw(simpangan, viewport, coefficient) + sisa.abs();
    let baru = rubber_band_offset(mentah, viewport, coefficient);
    if baru.is_finite() {
        batas + baru * arah
    } else {
        batas + overscroll_limit(viewport, coefficient) * arah
    }
}

// ---------------------------------------------------------------------------
// Scrollbar
// ---------------------------------------------------------------------------

/// Scrollbar thumb geometry along the scroll axis, in container-local
/// coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thumb {
    /// Distance from the container's leading edge to the start of the thumb.
    pub offset: f32,
    /// Thumb length along the scroll axis.
    pub length: f32,
}

impl Thumb {
    /// The far end of the thumb.
    pub fn end(self) -> f32 {
        self.offset + self.length
    }

    /// True when `pos` (a scroll-axis coordinate) falls on the thumb.
    pub fn contains(self, pos: f32) -> bool {
        pos >= self.offset && pos <= self.end()
    }
}

/// Thumb geometry for a given scroll state.
///
/// `None` means there is nothing to scroll — and therefore that no scrollbar
/// may be drawn at all.
///
/// Two behaviours borrowed from macOS:
///
/// - Thumb length is proportional to the **visible fraction of the content**,
///   but never shorter than `min_length` (hit target, HIG).
/// - When the content stretches past an edge, the thumb **shrinks** while
///   staying stuck to that edge — the feedback that makes rubber banding read
///   as deliberate rather than as a bug.
pub fn thumb(viewport: f32, content: f32, offset: f32, min_length: f32) -> Option<Thumb> {
    if viewport <= 0.0 || content <= viewport + EPS {
        return None;
    }
    let max = max_scroll(viewport, content);
    let minimum = min_length.clamp(0.0, viewport);
    let ideal = viewport * (viewport / content);
    let mut length = ideal.clamp(minimum, viewport);

    let simpangan = overshoot(offset, max).abs();
    if simpangan > 0.0 {
        length = (length - simpangan).max(minimum);
    }

    let jalur = (viewport - length).max(0.0);
    let posisi = if offset < 0.0 {
        0.0
    } else if offset > max {
        jalur
    } else if max <= 0.0 {
        0.0
    } else {
        (offset / max) * jalur
    };
    Some(Thumb {
        offset: posisi.clamp(0.0, jalur),
        length,
    })
}

/// Inverse of [`thumb`]: a dragged thumb position → a scroll position.
///
/// Used when the user drags the scrollbar directly. Always in bounds —
/// dragging the bar never produces a rubber band, exactly as in AppKit.
pub fn scroll_for_thumb(viewport: f32, content: f32, thumb_offset: f32, min_length: f32) -> f32 {
    let Some(t) = thumb(viewport, content, 0.0, min_length) else {
        return 0.0;
    };
    let jalur = (viewport - t.length).max(0.0);
    if jalur <= 0.0 {
        return 0.0;
    }
    let max = max_scroll(viewport, content);
    clamp_scroll(thumb_offset / jalur * max, max)
}

// ---------------------------------------------------------------------------
// Keyboard steps & scroll-to
// ---------------------------------------------------------------------------

/// How far a single Page Up/Down scrolls.
///
/// One full screen **minus one line**: the same convention as macOS, Windows,
/// and every browser — the eye needs a line of overlap to find its place again.
pub fn page_step(viewport: f32, line: f32) -> f32 {
    (viewport - line.max(0.0)).max(viewport * 0.5).max(0.0)
}

/// The smallest scroll position that makes the range `[start, start + extent]`
/// fully visible, with `padding` at its edges.
///
/// Already visible = do not move at all. That rule is what keeps
/// `scroll_into_view` from hopping about as focus moves between items that are
/// already both on screen.
pub fn scroll_to_reveal(offset: f32, viewport: f32, start: f32, extent: f32, padding: f32) -> f32 {
    let atas = start - padding;
    let bawah = start + extent.max(0.0) + padding;
    if atas < offset {
        atas
    } else if bawah > offset + viewport {
        // Content taller than the viewport is aligned to its leading edge:
        // seeing the start of a long item is always more useful than
        // seeing its end.
        (bawah - viewport).min(atas)
    } else {
        offset
    }
}

/// Scroll velocity from a pair of event samples, in logical points per second.
///
/// Trackpad inertia arrives from the OS as a stream of deltas (INTEGRASI-NATIVE
/// §3), not as a velocity. This turns it back into one so it can be handed off
/// to the spring when the content hits an edge — the handoff promised by §3.5,
/// only with a different source than a finger.
pub fn velocity_from(delta: f32, dt: std::time::Duration) -> f32 {
    let dt = dt.as_secs_f32();
    if dt <= 0.0 || !delta.is_finite() {
        return 0.0;
    }
    delta / dt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const VIEWPORT: f32 = 400.0;

    #[test]
    fn max_scroll_nol_saat_isi_muat() {
        assert_eq!(max_scroll(400.0, 300.0), 0.0);
        assert_eq!(max_scroll(400.0, 400.0), 0.0);
        assert_eq!(max_scroll(400.0, 1000.0), 600.0);
    }

    #[test]
    fn simpangan_hanya_di_luar_batas() {
        assert_eq!(overshoot(0.0, 600.0), 0.0);
        assert_eq!(overshoot(300.0, 600.0), 0.0);
        assert_eq!(overshoot(-20.0, 600.0), -20.0);
        assert_eq!(overshoot(640.0, 600.0), 40.0);
        assert_eq!(nearest_bound(-20.0, 600.0), 0.0);
        assert_eq!(nearest_bound(640.0, 600.0), 600.0);
    }

    #[test]
    fn faktor_rubber_band_turun_dari_satu_ke_nol() {
        let f0 = rubber_band_factor(0.0, VIEWPORT, RUBBER_BAND);
        assert!((f0 - 1.0).abs() < 1e-6, "di tepi harus bebas: {f0}");

        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        assert_eq!(rubber_band_factor(limit, VIEWPORT, RUBBER_BAND), 0.0);
        assert_eq!(rubber_band_factor(limit * 2.0, VIEWPORT, RUBBER_BAND), 0.0);

        // Monotonically decreasing, and never outside 0..1.
        let mut sebelumnya = f0;
        for i in 1..=40 {
            let f = rubber_band_factor(limit * i as f32 / 40.0, VIEWPORT, RUBBER_BAND);
            assert!((0.0..=1.0).contains(&f), "faktor liar: {f}");
            assert!(f <= sebelumnya, "faktor naik di langkah {i}");
            sebelumnya = f;
        }
        // The sign of the displacement makes no difference: stretching up and
        // stretching down behave identically.
        assert_eq!(
            rubber_band_factor(-30.0, VIEWPORT, RUBBER_BAND),
            rubber_band_factor(30.0, VIEWPORT, RUBBER_BAND)
        );
    }

    #[test]
    fn viewport_nol_tidak_pernah_menghasilkan_nan() {
        assert_eq!(rubber_band_factor(10.0, 0.0, RUBBER_BAND), 0.0);
        assert_eq!(rubber_band_offset(10.0, 0.0, RUBBER_BAND), 0.0);
        assert_eq!(apply_delta(0.0, 50.0, 0.0, 0.0, RUBBER_BAND), 0.0);
        assert!(thumb(0.0, 1000.0, 0.0, 44.0).is_none());
    }

    #[test]
    fn simpangan_tidak_pernah_melewati_batasnya() {
        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        assert_eq!(limit, VIEWPORT * RUBBER_BAND);
        // Any pull, however large, stops at the limit.
        assert!(rubber_band_offset(1.0e6, VIEWPORT, RUBBER_BAND) < limit);
        assert!(rubber_band_offset(1.0e6, VIEWPORT, RUBBER_BAND) > limit * 0.99);
        // The sign follows the direction of the pull.
        assert!(rubber_band_offset(-100.0, VIEWPORT, RUBBER_BAND) < 0.0);
        // Nearly 1:1 at first — the content must feel stuck to the finger.
        let kecil = rubber_band_offset(1.0, VIEWPORT, RUBBER_BAND);
        assert!(kecil > 0.99 && kecil < 1.0, "{kecil}");
    }

    /// The per-step factor and the closed form must be **the same formula**:
    /// summing up small steps has to land on `f(x)`.
    #[test]
    fn integral_faktor_sama_dengan_bentuk_tertutup() {
        let langkah = 0.05f32;
        let mut pos = 0.0f32;
        let mut tarik = 0.0f32;
        while tarik < 200.0 {
            pos = apply_delta(pos, -langkah, 0.0, VIEWPORT, RUBBER_BAND);
            tarik += langkah;
        }
        let tutup = rubber_band_offset(-tarik, VIEWPORT, RUBBER_BAND);
        assert!(
            (pos - tutup).abs() < 0.5,
            "integral {pos} vs bentuk tertutup {tutup}"
        );
    }

    #[test]
    fn jarak_tarik_dan_simpangan_bolak_balik() {
        for raw in [0.0f32, 3.0, 40.0, 500.0] {
            let y = rubber_band_offset(raw, VIEWPORT, RUBBER_BAND);
            let kembali = rubber_band_raw(y, VIEWPORT, RUBBER_BAND);
            assert!((kembali - raw).abs() < 0.01, "{raw} -> {y} -> {kembali}");
        }
        // At the limit (and beyond) the pull is infinite — which is the right
        // answer, and `apply_delta` handles it as "stop at the limit".
        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        assert!(rubber_band_raw(limit, VIEWPORT, RUBBER_BAND).is_infinite());
        assert!(rubber_band_raw(-3.0, VIEWPORT, RUBBER_BAND) < 0.0);
    }

    #[test]
    fn redaman_tidak_bergantung_ukuran_langkah() {
        // One 100-point step must land in the same place as a hundred 1-point
        // steps. This is what breaks if the per-step factor is applied as-is
        // to deltas the size of a scroll event.
        let max = 600.0;
        let sekali = apply_delta(600.0, 100.0, max, VIEWPORT, RUBBER_BAND);
        let mut bertahap = 600.0;
        for _ in 0..100 {
            bertahap = apply_delta(bertahap, 1.0, max, VIEWPORT, RUBBER_BAND);
        }
        assert!(
            (sekali - bertahap).abs() < 0.01,
            "sekali {sekali} vs bertahap {bertahap}"
        );
    }

    #[test]
    fn di_dalam_batas_bergerak_satu_banding_satu() {
        let max = 600.0;
        assert_eq!(apply_delta(0.0, 120.0, max, VIEWPORT, RUBBER_BAND), 120.0);
        assert_eq!(apply_delta(120.0, -50.0, max, VIEWPORT, RUBBER_BAND), 70.0);
        // Right up to the edge with no damping.
        assert_eq!(apply_delta(590.0, 10.0, max, VIEWPORT, RUBBER_BAND), 600.0);
    }

    #[test]
    fn melewati_tepi_langsung_teredam() {
        let max = 600.0;
        let baru = apply_delta(590.0, 60.0, max, VIEWPORT, RUBBER_BAND);
        assert!(baru > 600.0, "harus melar melewati tepi: {baru}");
        assert!(
            baru < 650.0,
            "50 poin sisanya harus teredam, bukan bergerak penuh: {baru}"
        );
    }

    #[test]
    fn kembali_dari_simpangan_tidak_pernah_diredam() {
        let max = 600.0;
        // Stretched 40 points past the top; a 40-point pull must land exactly on 0.
        assert_eq!(apply_delta(-40.0, 40.0, max, VIEWPORT, RUBBER_BAND), 0.0);
        // Anything beyond that: the remainder enters normal territory, still 1:1.
        assert_eq!(apply_delta(-40.0, 60.0, max, VIEWPORT, RUBBER_BAND), 20.0);
        // Symmetric at the bottom end.
        assert_eq!(apply_delta(640.0, -40.0, max, VIEWPORT, RUBBER_BAND), 600.0);
    }

    #[test]
    fn tarikan_gila_tetap_berhenti_di_limit() {
        let max = 600.0;
        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        let atas = apply_delta(0.0, -1.0e9, max, VIEWPORT, RUBBER_BAND);
        assert!(atas >= -limit && atas <= 0.0, "{atas}");
        let bawah = apply_delta(max, 1.0e9, max, VIEWPORT, RUBBER_BAND);
        assert!(bawah <= max + limit, "{bawah}");
        assert!(apply_delta(0.0, f32::NAN, max, VIEWPORT, RUBBER_BAND) == 0.0);
    }

    #[test]
    fn thumb_tidak_ada_saat_isi_muat() {
        assert!(thumb(400.0, 400.0, 0.0, 44.0).is_none());
        assert!(thumb(400.0, 200.0, 0.0, 44.0).is_none());
        assert!(thumb(400.0, 401.0, 0.0, 44.0).is_some());
    }

    #[test]
    fn panjang_thumb_sebanding_porsi_terlihat() {
        let t = thumb(400.0, 800.0, 0.0, 10.0).expect("bisa digulir");
        assert!((t.length - 200.0).abs() < 1e-3, "{t:?}");
        assert_eq!(t.offset, 0.0);

        // At the bottom end the thumb sits flush with the trailing edge.
        let t = thumb(400.0, 800.0, 400.0, 10.0).expect("bisa digulir");
        assert!((t.end() - 400.0).abs() < 1e-3, "{t:?}");

        // Halfway down, exactly halfway along its track.
        let t = thumb(400.0, 800.0, 200.0, 10.0).expect("bisa digulir");
        assert!((t.offset - 100.0).abs() < 1e-3, "{t:?}");
    }

    #[test]
    fn thumb_tidak_pernah_lebih_pendek_dari_hit_target() {
        // Content 100× the viewport: proportional sizing would give 4 points —
        // undraggable by anyone.
        let t = thumb(400.0, 40_000.0, 0.0, 44.0).expect("bisa digulir");
        assert!(t.length >= 44.0, "{t:?}");
        // A viewport shorter than the hit target still gives a sane result.
        let t = thumb(30.0, 300.0, 0.0, 44.0).expect("bisa digulir");
        assert!(t.length <= 30.0, "{t:?}");
    }

    #[test]
    fn thumb_menyusut_saat_isi_melar() {
        let normal = thumb(400.0, 800.0, 0.0, 10.0).expect("bisa digulir");
        let melar = thumb(400.0, 800.0, -60.0, 10.0).expect("bisa digulir");
        assert!(melar.length < normal.length, "{melar:?} vs {normal:?}");
        assert_eq!(melar.offset, 0.0, "menempel di tepi yang dilewati");

        let bawah = thumb(400.0, 800.0, 460.0, 10.0).expect("bisa digulir");
        assert!(bawah.length < normal.length);
        assert!((bawah.end() - 400.0).abs() < 1e-3, "{bawah:?}");
        // Never shrinks below the hit target.
        let ekstrem = thumb(400.0, 800.0, -1000.0, 44.0).expect("bisa digulir");
        assert!(ekstrem.length >= 44.0);
    }

    #[test]
    fn thumb_dan_kebalikannya_bolak_balik() {
        for offset in [0.0f32, 37.0, 200.0, 399.5, 400.0] {
            let t = thumb(400.0, 800.0, offset, 44.0).expect("bisa digulir");
            let kembali = scroll_for_thumb(400.0, 800.0, t.offset, 44.0);
            assert!(
                (kembali - offset).abs() < 0.01,
                "{offset} -> {t:?} -> {kembali}"
            );
        }
        // Drags outside the track are clamped, not turned into wild positions.
        assert_eq!(scroll_for_thumb(400.0, 800.0, -100.0, 44.0), 0.0);
        assert_eq!(scroll_for_thumb(400.0, 800.0, 10_000.0, 44.0), 400.0);
        assert_eq!(scroll_for_thumb(400.0, 300.0, 50.0, 44.0), 0.0);
    }

    #[test]
    fn satu_halaman_menyisakan_satu_baris_tumpang_tindih() {
        assert_eq!(page_step(400.0, 20.0), 380.0);
        // A tiny viewport must never produce a zero or negative step.
        assert_eq!(page_step(30.0, 20.0), 15.0);
        assert!(page_step(0.0, 20.0) >= 0.0);
    }

    #[test]
    fn scroll_to_reveal_diam_bila_sudah_terlihat() {
        // An item in the middle of the viewport: no reason to move.
        assert_eq!(scroll_to_reveal(100.0, 400.0, 200.0, 40.0, 8.0), 100.0);
        // Above the screen → scroll up to the top edge + padding.
        assert_eq!(scroll_to_reveal(300.0, 400.0, 100.0, 40.0, 8.0), 92.0);
        // Below the screen → scroll down until its end fits + padding.
        assert_eq!(scroll_to_reveal(0.0, 400.0, 500.0, 40.0, 8.0), 148.0);
        // Content taller than the viewport: its start takes priority.
        assert_eq!(scroll_to_reveal(0.0, 100.0, 50.0, 400.0, 0.0), 50.0);
    }

    #[test]
    fn kecepatan_dari_dua_sampel() {
        assert_eq!(velocity_from(30.0, Duration::from_millis(10)), 3000.0);
        assert_eq!(velocity_from(30.0, Duration::ZERO), 0.0);
        assert_eq!(velocity_from(f32::NAN, Duration::from_millis(10)), 0.0);
    }
}
