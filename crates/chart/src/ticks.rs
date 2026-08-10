//! **Ticks** — where the gridlines go, and what the labels say.
//!
//! Tick placement is the part of a chart that most obviously looks wrong when
//! it is done naively. Slicing the domain into *n* equal parts gives axis
//! labels like `0`, `3.7143`, `7.4286` — arithmetically impeccable and useless
//! to a reader, who is here to compare magnitudes at a glance, not to admire
//! division.
//!
//! So ticks land on **round numbers**: the step is snapped to 1, 2, 2.5, 5, or
//! 10 times a power of ten, and the domain is then *widened* to the enclosing
//! round numbers so the topmost gridline is a real value rather than the
//! maximum of whatever happened to be measured. That widening is why
//! [`nice_domain`] exists and why the axis, not the data, decides where the
//! plot ends.
//!
//! Time gets its own path ([`time_ticks`]) for the same reason. Months are not
//! 30 days, years are not 365, and a "round number of days" is a meaningless
//! unit to a human: an axis spanning two years wants January marks, not marks
//! every 512 days.
//!
//! ```
//! use silka_chart::ticks::{nice_domain, nice_ticks};
//!
//! // A messy domain becomes readable numbers…
//! assert_eq!(nice_ticks(0.0, 96.3, 5), vec![0.0, 25.0, 50.0, 75.0, 100.0]);
//! // …and the axis grows to enclose them, so the top gridline is labelled.
//! assert_eq!(nice_domain(0.0, 96.3, 5), (0.0, 100.0));
//! ```

use crate::date::Date;

/// How many logical points one tick wants for itself when its labels sit
/// **side by side** along the axis.
///
/// This is the case for any horizontal axis: neighbouring labels collide
/// *widthwise*, and a label is far wider than it is tall. The number is the
/// framework's, not the platform's — the same reasoning as [`ClickConfig`](silka_core::input::ClickConfig)
/// (`silka_core::input`): readers expect the same density on every OS.
pub const MIN_TICK_SPACING: f32 = 48.0;

/// How many logical points one tick wants when its labels are **stacked**
/// along the axis.
///
/// The case for any vertical axis, and the reason it needs its own number: a
/// stacked label collides *heightwise*, and a label is roughly a third as tall
/// as it is wide. Using the side-by-side figure here — the mistake that is easy
/// to make because there is only one obvious constant — leaves a 130pt tall
/// plot with exactly two gridlines, which is an axis in name only.
pub const MIN_STACKED_TICK_SPACING: f32 = 28.0;

/// How many ticks fit along an axis of `extent` points.
///
/// Clamped to at least two: an axis with one tick has no scale, it has a label.
pub fn tick_count_for(extent: f32, min_spacing: f32) -> usize {
    if !extent.is_finite() || extent <= 0.0 || min_spacing <= 0.0 {
        return 2;
    }
    ((extent / min_spacing).floor() as usize).clamp(2, 12)
}

/// The "nice" step at or above `raw`, from the 1 / 2 / 2.5 / 5 / 10 family.
///
/// The 2.5 is not decoration: without it, a domain wanting a step near 2.5
/// jumps to 5 and halves the number of gridlines, which reads as a much
/// coarser chart than the caller asked for.
pub fn nice_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let stepped = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 2.5 {
        2.5
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    stepped * magnitude
}

/// Tick values across `[min, max]`, on round numbers, roughly `target` of them.
///
/// "Roughly" is deliberate. A tick generator that hits an exact count has to
/// give up round numbers to do it, and round numbers are the entire point.
pub fn nice_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    let (min, max) = ordered(min, max);
    let target = target.max(2);
    if !min.is_finite() || !max.is_finite() {
        return Vec::new();
    }
    if (max - min).abs() < f64::EPSILON {
        return vec![min];
    }
    let step = nice_step((max - min) / (target - 1) as f64);
    let start = (min / step).floor() * step;
    let end = (max / step).ceil() * step;

    let count = ((end - start) / step).round() as i64;
    // A step that cannot subdivide the domain means something is degenerate
    // (an infinite domain, a subnormal step); returning the two endpoints is
    // still a usable axis, an empty vector is not.
    if count <= 0 || count > 1_000 {
        return vec![min, max];
    }
    (0..=count)
        .map(|i| clean(start + step * i as f64, step))
        .collect()
}

/// The domain widened to the round numbers that enclose it.
///
/// Called before building the [`LinearScale`](crate::scale::LinearScale), so
/// the first and last gridlines sit exactly on the plot edges rather than
/// floating just inside them.
pub fn nice_domain(min: f64, max: f64, target: usize) -> (f64, f64) {
    let ticks = nice_ticks(min, max, target);
    match (ticks.first(), ticks.last()) {
        (Some(lo), Some(hi)) if hi > lo => (*lo, *hi),
        _ => ordered(min, max),
    }
}

/// A domain for a value axis that **includes zero**.
///
/// The rule for bars, and it is not a preference: a bar's length *is* its
/// value, so a bar axis that starts at 40 draws a bar twice as long for a value
/// 5% larger. Lines and areas are exempt — position, not length, carries their
/// meaning, and forcing zero onto a series that lives between 980 and 1010
/// flattens it into a straight line.
pub fn zero_based_domain(min: f64, max: f64, target: usize) -> (f64, f64) {
    let (min, max) = ordered(min, max);
    nice_domain(min.min(0.0), max.max(0.0), target)
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// The granularity a time axis is labelled at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    /// Individual days.
    Day,
    /// Whole weeks, snapped to Monday.
    Week,
    /// Whole months, snapped to the 1st.
    Month,
    /// Whole quarters.
    Quarter,
    /// Whole years.
    Year,
}

impl TimeUnit {
    /// The unit that gives roughly `target` labels over a span of `days`.
    ///
    /// The thresholds are the ones a reader would pick by hand: a fortnight
    /// wants day marks, a quarter wants weeks, two years want months.
    pub fn for_span(days: f64, target: usize) -> TimeUnit {
        let target = target.max(2) as f64;
        let per_tick = days / target;
        if per_tick <= 1.5 {
            TimeUnit::Day
        } else if per_tick <= 10.0 {
            TimeUnit::Week
        } else if per_tick <= 45.0 {
            TimeUnit::Month
        } else if per_tick <= 130.0 {
            TimeUnit::Quarter
        } else {
            TimeUnit::Year
        }
    }

    /// A short name for debug output and tests.
    pub const fn name(self) -> &'static str {
        match self {
            TimeUnit::Day => "day",
            TimeUnit::Week => "week",
            TimeUnit::Month => "month",
            TimeUnit::Quarter => "quarter",
            TimeUnit::Year => "year",
        }
    }
}

/// Tick positions on a time axis, as **day numbers** (see [`crate::date`]).
///
/// Every tick lands on a boundary of its unit: the 1st of the month, the
/// Monday of the week, the 1st of January. A tick in the middle of a month
/// labelled "August" would be a lie about where August starts.
pub fn time_ticks(min_days: f64, max_days: f64, target: usize) -> (TimeUnit, Vec<f64>) {
    let (min_days, max_days) = ordered(min_days, max_days);
    if !min_days.is_finite() || !max_days.is_finite() {
        return (TimeUnit::Day, Vec::new());
    }
    let unit = TimeUnit::for_span(max_days - min_days, target);
    let lo = min_days.floor() as i64;
    let hi = max_days.ceil() as i64;
    let mut out = Vec::new();

    match unit {
        TimeUnit::Day => {
            // Days still need a stride: 30 days over a narrow chart is 30
            // labels on top of each other.
            let stride = ((hi - lo + 1) as f64 / target.max(2) as f64)
                .ceil()
                .max(1.0) as i64;
            let mut d = lo;
            while d <= hi {
                out.push(d as f64);
                d += stride;
            }
        }
        TimeUnit::Week => {
            let stride = (((hi - lo) as f64 / 7.0 / target.max(2) as f64)
                .ceil()
                .max(1.0) as i64)
                * 7;
            let mut d = Date::from_days(lo).start_of_week().to_days();
            if d < lo {
                d += 7;
            }
            while d <= hi {
                out.push(d as f64);
                d += stride;
            }
        }
        TimeUnit::Month | TimeUnit::Quarter | TimeUnit::Year => {
            let langkah = match unit {
                TimeUnit::Month => 1,
                TimeUnit::Quarter => 3,
                _ => 12,
            };
            let awal = match unit {
                TimeUnit::Month => Date::from_days(lo).start_of_month(),
                TimeUnit::Quarter => Date::from_days(lo).start_of_quarter(),
                _ => Date::from_days(lo).start_of_year(),
            };
            // How many units per tick, so a five-year span does not draw sixty
            // month marks.
            let total = ((hi - lo) as f64 / (langkah as f64 * 30.44))
                .ceil()
                .max(1.0);
            let stride = (total / target.max(2) as f64).ceil().max(1.0) as i32 * langkah;
            let mut tanggal = awal;
            if tanggal.to_days() < lo {
                tanggal = tanggal.add_months(stride);
            }
            while tanggal.to_days() <= hi {
                out.push(tanggal.to_days() as f64);
                tanggal = tanggal.add_months(stride);
            }
        }
    }
    (unit, out)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ordered(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Round away the floating-point dust that accumulates in `start + step * i`.
///
/// Without this an axis prints `0.30000000000000004`, which is both true and
/// unforgivable.
fn clean(value: f64, step: f64) -> f64 {
    if step <= 0.0 || !step.is_finite() {
        return value;
    }
    let digits = (-step.log10().floor()).clamp(0.0, 12.0) as i32;
    let factor = 10f64.powi(digits + 1);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn langkah_dijepret_ke_keluarga_bulat() {
        assert_eq!(nice_step(0.9), 1.0);
        assert_eq!(nice_step(1.1), 2.0);
        assert_eq!(nice_step(2.3), 2.5);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(6.0), 10.0);
        // Scale invariance: the family repeats at every power of ten.
        assert_eq!(nice_step(230.0), 250.0);
        assert_eq!(nice_step(0.023), 0.025);
    }

    #[test]
    fn langkah_tidak_masuk_akal_tidak_panik() {
        assert_eq!(nice_step(0.0), 1.0);
        assert_eq!(nice_step(-5.0), 1.0);
        assert_eq!(nice_step(f64::NAN), 1.0);
    }

    #[test]
    fn tick_adalah_angka_yang_bisa_dibaca_manusia() {
        // The whole reason this module exists: a naive generator would answer
        // 0, 24.075, 48.15… here.
        assert_eq!(nice_ticks(0.0, 96.3, 5), vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        assert_eq!(
            nice_ticks(0.0, 10.0, 6),
            vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
    }

    #[test]
    fn tick_melingkupi_domain_bukan_memotongnya() {
        let t = nice_ticks(3.0, 87.0, 5);
        assert!(t[0] <= 3.0, "{t:?}");
        assert!(*t.last().unwrap() >= 87.0, "{t:?}");
        assert_eq!(nice_domain(3.0, 87.0, 5), (0.0, 100.0));
    }

    #[test]
    fn tick_naik_dengan_jarak_seragam() {
        for (lo, hi) in [
            (0.0, 1.0),
            (-50.0, 50.0),
            (1_000.0, 9_500.0),
            (0.001, 0.009),
        ] {
            let t = nice_ticks(lo, hi, 5);
            assert!(t.len() >= 2, "{lo}..{hi}: {t:?}");
            let langkah = t[1] - t[0];
            for w in t.windows(2) {
                assert!(
                    ((w[1] - w[0]) - langkah).abs() < langkah * 1e-6,
                    "{lo}..{hi}: {t:?}"
                );
            }
        }
    }

    #[test]
    fn tick_bebas_dari_debu_floating_point() {
        // 0.1 + 0.2 arithmetic must not reach the axis label.
        let t = nice_ticks(0.0, 1.0, 6);
        for v in &t {
            let teks = format!("{v}");
            assert!(teks.len() <= 4, "label {teks} membawa debu floating point");
        }
    }

    #[test]
    fn domain_negatif_dan_melintasi_nol() {
        let t = nice_ticks(-37.0, 12.0, 5);
        assert!(
            t.contains(&0.0),
            "nol harus jadi tick saat domain melintasinya: {t:?}"
        );
        assert!(t[0] <= -37.0 && *t.last().unwrap() >= 12.0);
    }

    #[test]
    fn domain_datar_menghasilkan_satu_tick() {
        assert_eq!(nice_ticks(5.0, 5.0, 5), vec![5.0]);
    }

    #[test]
    fn domain_tak_hingga_tidak_menggantung() {
        // A NaN in the data must not turn into an infinite loop inside layout.
        assert!(nice_ticks(f64::NAN, 10.0, 5).is_empty());
        assert!(nice_ticks(0.0, f64::INFINITY, 5).is_empty());
    }

    #[test]
    fn sumbu_batang_selalu_menyertakan_nol() {
        // Bars encode value as length, so a truncated axis is a lie about
        // magnitude — the one axis rule this crate enforces rather than offers.
        let (lo, hi) = zero_based_domain(980.0, 1_010.0, 5);
        assert_eq!(lo, 0.0);
        assert!(hi >= 1_010.0);
        // Negative values keep their side of zero.
        let (lo, hi) = zero_based_domain(-40.0, -5.0, 5);
        assert!(lo <= -40.0 && hi == 0.0);
    }

    #[test]
    fn jumlah_tick_mengikuti_ruang_yang_ada() {
        assert_eq!(tick_count_for(480.0, MIN_TICK_SPACING), 10);
        assert_eq!(
            tick_count_for(60.0, MIN_TICK_SPACING),
            2,
            "sumbu sempit tetap punya dua"
        );
        assert_eq!(tick_count_for(0.0, MIN_TICK_SPACING), 2);
        assert_eq!(tick_count_for(f32::NAN, MIN_TICK_SPACING), 2);
    }

    // -- time ---------------------------------------------------------------

    fn tanggal(y: i32, m: u32, d: u32) -> f64 {
        Date::new(y, m, d).to_days() as f64
    }

    #[test]
    fn satuan_waktu_mengikuti_rentang() {
        assert_eq!(TimeUnit::for_span(7.0, 5), TimeUnit::Day);
        assert_eq!(TimeUnit::for_span(40.0, 5), TimeUnit::Week);
        assert_eq!(TimeUnit::for_span(365.0, 12), TimeUnit::Month);
        // A year with room for only five marks wants quarters, not months —
        // twelve labels crammed into five slots is how axes become unreadable.
        assert_eq!(TimeUnit::for_span(365.0, 5), TimeUnit::Quarter);
        assert_eq!(TimeUnit::for_span(365.0 * 8.0, 5), TimeUnit::Year);
    }

    #[test]
    fn tick_bulanan_jatuh_di_tanggal_satu() {
        // A tick labelled "August" that sits on 12 August is a lie about where
        // August begins.
        let (unit, ticks) = time_ticks(tanggal(2026, 1, 12), tanggal(2026, 12, 20), 12);
        assert_eq!(unit, TimeUnit::Month);
        assert!(!ticks.is_empty());
        for t in &ticks {
            let d = Date::from_days(*t as i64);
            assert_eq!(d.day, 1, "{d:?}");
        }
    }

    #[test]
    fn tick_pekanan_jatuh_di_hari_senin() {
        let (unit, ticks) = time_ticks(tanggal(2026, 8, 3), tanggal(2026, 9, 14), 6);
        assert_eq!(unit, TimeUnit::Week);
        for t in &ticks {
            assert_eq!(Date::from_days(*t as i64).weekday(), 0, "{t}");
        }
    }

    #[test]
    fn tick_tahunan_jatuh_di_januari() {
        let (unit, ticks) = time_ticks(tanggal(2015, 6, 1), tanggal(2026, 6, 1), 5);
        assert_eq!(unit, TimeUnit::Year);
        for t in &ticks {
            let d = Date::from_days(*t as i64);
            assert_eq!((d.month, d.day), (1, 1), "{d:?}");
        }
    }

    #[test]
    fn tick_waktu_tetap_di_dalam_rentang_dan_tidak_meledak() {
        for (a, b) in [
            (tanggal(2026, 1, 1), tanggal(2026, 1, 8)),
            (tanggal(2020, 1, 1), tanggal(2026, 8, 10)),
            (tanggal(1990, 3, 5), tanggal(2026, 8, 10)),
        ] {
            let (_, ticks) = time_ticks(a, b, 6);
            assert!(!ticks.is_empty(), "{a}..{b}");
            assert!(ticks.len() <= 40, "{} tick terlalu banyak", ticks.len());
            for t in &ticks {
                assert!(*t >= a.floor() && *t <= b.ceil(), "{t} di luar {a}..{b}");
            }
            assert!(ticks.windows(2).all(|w| w[1] > w[0]), "tick harus menaik");
        }
    }
}
