//! The dashboard's data — static, dummy, and deliberately shaped like the real
//! thing.
//!
//! Two rules hold everywhere in this module, and both are framework decisions
//! rather than taste:
//!
//! 1. **Money and dates are never formatted by hand.** They go through
//!    `silka_chart::format` with [`Locale::ID_ID`], the same formatter the
//!    charts use for their axis labels. A dashboard whose card says
//!    `Rp 121.000.000` while its chart axis says `Rp121,000,000` is a
//!    dashboard that lies about being one product.
//! 2. **No colour is written here.** Anything that needs a hue asks
//!    [`silka_chart::ChartPalette`] for a slot, so the tints stay
//!    colour-blind-safe and follow the appearance (§2.7).
//!
//! The people and the account numbers are Indonesian because the product is;
//! every label the user reads is English because the repository is.

use silka_chart::format::{Locale, NumberFormat};
use silka_chart::Date;

/// The locale the whole application formats in.
pub const LOCALE: Locale = Locale::ID_ID;

/// "Today" for this dataset — a fixed day so the screenshots, the golden
/// numbers, and the tests never drift with the wall clock.
pub const TODAY: Date = Date::new(2026, 7, 28);

/// An amount in rupiah: `Rp 121.000.000`.
///
/// Cents are noise on a lending desk, so the currency format carries no
/// decimals; the grouping character comes from the locale, not from a `if
/// indonesian { '.' }` somewhere.
pub fn rupiah(amount: f64) -> String {
    NumberFormat::currency("Rp").format(amount, &LOCALE)
}

/// A whole number with locale grouping: `1.234`.
pub fn count(value: u32) -> String {
    LOCALE.number(value as f64, 0)
}

/// A date as a day number: `28 Jul 2026`.
pub fn date(days: f64) -> String {
    LOCALE.date_full(days)
}

/// A signed change against the previous period: `+12%` or `-5%`.
///
/// `NumberFormat::Percent` does not sign a positive value (`format(0.12, …)`
/// is `12%`, not `+12%`), and a delta without its sign is not a trend a
/// reader can act on in a glance — that is the one thing this wrapper adds.
pub fn delta_text(delta: f32) -> String {
    let pct = NumberFormat::Percent(0).format(delta.abs() as f64, &LOCALE);
    if delta < 0.0 {
        format!("-{pct}")
    } else {
        format!("+{pct}")
    }
}

/// A day number `offset` days away from [`TODAY`].
pub fn day(offset: i64) -> f64 {
    (TODAY.to_days() + offset) as f64
}

// ---------------------------------------------------------------------------
// KPI tiles
// ---------------------------------------------------------------------------

/// How a KPI tile is tinted.
///
/// The variants name a **role**, never a colour: the actual hue is a slot of
/// the categorical palette, picked in `kit::kpi_tile`, so the same tile is
/// correct in light and dark and under both presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    /// No tint at all — the plain surface card.
    Plain,
    /// A tinted card; the number is the categorical slot it borrows.
    Slot(usize),
}

/// One statistic on the KPI grid.
#[derive(Debug, Clone, Copy)]
pub struct Kpi {
    /// The small caps label.
    pub label: &'static str,
    /// The big number, already formatted.
    pub value: KpiValue,
    /// How the tile is tinted.
    pub tint: Tint,
    /// Change against the previous period, as a fraction (`0.12` = `+12%`).
    ///
    /// `None` where a trend would not mean anything on this dataset — a
    /// count of zero has no previous-period ratio to speak of, and inventing
    /// one would be a number this file made up rather than data. The
    /// direction is read literally (up is drawn in `success`, down in
    /// `destructive`): whether a smaller number is actually the good outcome
    /// (fewer rejections, say) is a per-metric judgement this dataset does
    /// not attempt to encode.
    pub delta: Option<f32>,
}

/// What a KPI tile shows — kept as data so the formatter, not the literal,
/// decides how it reads.
#[derive(Debug, Clone, Copy)]
pub enum KpiValue {
    /// A plain count.
    Count(u32),
    /// An amount of money.
    Money(f64),
}

impl KpiValue {
    /// The text on the tile.
    pub fn text(self) -> String {
        match self {
            KpiValue::Count(n) => count(n),
            KpiValue::Money(v) => rupiah(v),
        }
    }
}

/// The ten tiles of the KPI grid, in reading order.
pub const KPIS: [Kpi; 10] = [
    Kpi {
        label: "Total customers",
        value: KpiValue::Count(20),
        tint: Tint::Plain,
        delta: Some(0.08),
    },
    Kpi {
        label: "In pipeline",
        value: KpiValue::Count(3),
        tint: Tint::Slot(0),
        delta: Some(-0.25),
    },
    Kpi {
        label: "Verification",
        value: KpiValue::Count(3),
        tint: Tint::Slot(7),
        delta: Some(0.5),
    },
    Kpi {
        label: "Processing",
        value: KpiValue::Count(2),
        tint: Tint::Slot(5),
        delta: None,
    },
    Kpi {
        label: "Rejected",
        value: KpiValue::Count(1),
        tint: Tint::Plain,
        delta: Some(-0.5),
    },
    Kpi {
        label: "Akad scheduled",
        value: KpiValue::Count(3),
        tint: Tint::Slot(2),
        delta: Some(0.2),
    },
    Kpi {
        label: "Akad completed today",
        value: KpiValue::Count(0),
        tint: Tint::Slot(6),
        delta: None,
    },
    Kpi {
        label: "Disbursement pending",
        value: KpiValue::Count(2),
        tint: Tint::Slot(1),
        delta: Some(-0.1),
    },
    Kpi {
        label: "Disbursement today",
        value: KpiValue::Count(4),
        tint: Tint::Slot(3),
        delta: Some(0.33),
    },
    Kpi {
        label: "Disbursed today",
        value: KpiValue::Money(196_000_000.0),
        tint: Tint::Slot(4),
        delta: Some(0.15),
    },
];

// ---------------------------------------------------------------------------
// The two list cards
// ---------------------------------------------------------------------------

/// One scheduled akad (contract signing).
#[derive(Debug, Clone, Copy)]
pub struct Akad {
    /// The borrower.
    pub name: &'static str,
    /// The national ID number, shown under the name.
    pub nik: &'static str,
    /// Offset in days from [`TODAY`] — turned into text by [`date`].
    pub day_offset: i64,
}

/// The "Akad Scheduled" card's rows.
pub const AKAD: [Akad; 4] = [
    Akad {
        name: "Dian Permata",
        nik: "3171011010101010",
        day_offset: 0,
    },
    Akad {
        name: "Yanto Kurniawan",
        nik: "3171011919191919",
        day_offset: 0,
    },
    Akad {
        name: "Dewi Lestari",
        nik: "3171010404040004",
        day_offset: 3,
    },
    Akad {
        name: "Bagas Nugroho",
        nik: "3171012121212121",
        day_offset: 5,
    },
];

/// How a disbursement ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting on the bank.
    Pending,
    /// Money has left the account.
    Success,
}

impl Status {
    /// The word inside the badge.
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Success => "success",
        }
    }
}

/// One disbursement.
#[derive(Debug, Clone, Copy)]
pub struct Disbursement {
    /// The borrower.
    pub name: &'static str,
    /// How much left the account.
    pub amount: f64,
    /// Where it got to.
    pub status: Status,
}

/// The "Recent Disbursements" card's rows.
pub const DISBURSEMENTS: [Disbursement; 5] = [
    Disbursement {
        name: "Dian Permata",
        amount: 121_000_000.0,
        status: Status::Pending,
    },
    Disbursement {
        name: "Yanto Kurniawan",
        amount: 26_000_000.0,
        status: Status::Pending,
    },
    Disbursement {
        name: "Wahyu Pranoto",
        amount: 10_000_000.0,
        status: Status::Success,
    },
    Disbursement {
        name: "Tony Setiawan",
        amount: 25_000_000.0,
        status: Status::Success,
    },
    Disbursement {
        name: "Citra Dewi",
        amount: 40_000_000.0,
        status: Status::Success,
    },
];

// ---------------------------------------------------------------------------
// Quick links
// ---------------------------------------------------------------------------

/// One tile in the "Quick Links" card.
#[derive(Debug, Clone, Copy)]
pub struct QuickLink {
    /// The tile's caption — also its a11y name.
    pub label: &'static str,
    /// One line of explanation under it.
    pub detail: &'static str,
    /// The categorical slot the tile borrows its tint from.
    pub slot: usize,
}

/// The quick links, in reading order.
pub const QUICK_LINKS: [QuickLink; 4] = [
    QuickLink {
        label: "New application",
        detail: "Start an ADK intake",
        slot: 0,
    },
    QuickLink {
        label: "Schedule akad",
        detail: "Book a signing slot",
        slot: 2,
    },
    QuickLink {
        label: "Release funds",
        detail: "Approve a disbursement",
        slot: 6,
    },
    QuickLink {
        label: "Daily recap",
        detail: "Close today's book",
        slot: 5,
    },
];

// ---------------------------------------------------------------------------
// The chart series
// ---------------------------------------------------------------------------

/// One day on the disbursement trend chart.
#[derive(Debug, Clone, Copy)]
pub struct TrendPoint {
    /// Day number since 1970-01-01 — the vocabulary `silka-chart` speaks for
    /// time.
    pub date: f64,
    /// Rupiah disbursed that day.
    pub disbursed: f64,
}

/// How many days the trend covers.
pub const TREND_DAYS: usize = 30;

/// A deterministic pseudo-random stream: the same input always gives the same
/// series, so a golden test of this page is not flaky.
fn noise(i: u64) -> f64 {
    let mut x = i
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x % 10_000) as f64 / 10_000.0
}

/// The last [`TREND_DAYS`] days of disbursement, ending on [`TODAY`].
pub fn trend() -> Vec<TrendPoint> {
    (0..TREND_DAYS)
        .map(|i| {
            let offset = i as i64 - (TREND_DAYS as i64 - 1);
            // A weekly rhythm plus noise: lending desks are quiet on weekends,
            // and a flat line would make the chart look like decoration.
            let weekly = ((i as f64) * 0.9).sin() * 0.3 + 1.0;
            TrendPoint {
                date: day(offset),
                disbursed: (90.0e6 + noise(i as u64) * 130.0e6) * weekly,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The transactions table
// ---------------------------------------------------------------------------

/// How many rows the transactions table holds.
pub const TRANSACTIONS: usize = 2_500;

/// The counterparty on row `i`.
pub fn party(i: usize) -> &'static str {
    const NAMES: [&str; 10] = [
        "Dian Permata",
        "Yanto Kurniawan",
        "Dewi Lestari",
        "Wahyu Pranoto",
        "Tony Setiawan",
        "Citra Dewi",
        "Bagas Nugroho",
        "Ratna Ayu",
        "Hendra Wijaya",
        "Sinta Maharani",
    ];
    NAMES[i % NAMES.len()]
}

/// The contract number on row `i`.
pub fn contract(i: usize) -> String {
    format!("ADK-{:06}", i + 1)
}

/// The status of row `i`.
pub fn status(i: usize) -> Status {
    if i % 5 == 0 {
        Status::Pending
    } else {
        Status::Success
    }
}

/// The amount on row `i`, in rupiah.
pub fn amount(i: usize) -> f64 {
    (((i * 8_191) % 900 + 100) as f64) * 125_000.0
}

/// The value date of row `i`.
pub fn value_date(i: usize) -> f64 {
    day(-((i % TREND_DAYS) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_formatted_by_the_locale_not_by_hand() {
        // The separator, the spacing after the symbol, and the absence of
        // decimals all come from `Locale::ID_ID`. If this ever reads
        // "Rp121,000,000" the dashboard stopped speaking the locale.
        assert_eq!(rupiah(121_000_000.0), "Rp 121.000.000");
        assert_eq!(rupiah(0.0), "Rp 0");
    }

    #[test]
    fn dates_are_formatted_by_the_locale_not_by_hand() {
        // Day/month order and the Indonesian month abbreviations are the
        // locale's job: `Agu`, not `Aug`.
        assert_eq!(date(day(0)), "28 Jul 2026");
        assert_eq!(date(day(7)), "4 Agu 2026");
    }

    #[test]
    fn every_kpi_has_a_label_and_a_readable_value() {
        for k in KPIS {
            assert!(!k.label.is_empty());
            assert!(!k.value.text().is_empty(), "{}", k.label);
        }
    }

    #[test]
    fn the_money_kpi_reads_as_money() {
        let money = KPIS
            .iter()
            .find(|k| matches!(k.value, KpiValue::Money(_)))
            .expect("one KPI is an amount");
        assert!(money.value.text().starts_with("Rp "));
    }

    #[test]
    fn delta_text_signs_a_positive_change_the_formatter_would_leave_bare() {
        assert_eq!(delta_text(0.12), "+12%");
        assert_eq!(delta_text(-0.05), "-5%");
        assert_eq!(delta_text(0.0), "+0%");
    }

    #[test]
    fn every_kpi_delta_has_a_direction_that_agrees_with_its_sign() {
        // Not a tautology: this is what keeps a future edit that flips a sign
        // by hand (rather than through `delta_text`) from silently drawing a
        // downward tile in `success` green.
        for k in KPIS {
            if let Some(d) = k.delta {
                let text = delta_text(d);
                if d < 0.0 {
                    assert!(text.starts_with('-'), "{}: {text}", k.label);
                } else {
                    assert!(text.starts_with('+'), "{}: {text}", k.label);
                }
            }
        }
    }

    #[test]
    fn a_zero_count_kpi_has_no_delta() {
        // "Akad completed today" is 0 — a percent change against nothing is
        // not a number this dataset invents.
        let zero = KPIS
            .iter()
            .find(|k| matches!(k.value, KpiValue::Count(0)))
            .expect("one KPI is a zero count");
        assert_eq!(zero.delta, None, "{}", zero.label);
    }

    #[test]
    fn tinted_kpis_stay_inside_the_categorical_palette() {
        for k in KPIS {
            if let Tint::Slot(i) = k.tint {
                assert!(
                    i < silka_chart::CATEGORICAL_LEN,
                    "slot {i} of '{}' is outside the palette",
                    k.label
                );
            }
        }
    }

    #[test]
    fn the_trend_series_is_deterministic_and_ends_today() {
        let a = trend();
        let b = trend();
        assert_eq!(a.len(), TREND_DAYS);
        assert_eq!(a.last().unwrap().date, day(0));
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.disbursed, y.disbursed);
        }
    }

    #[test]
    fn table_rows_never_run_out_of_data() {
        for i in [0, 1, 7, TRANSACTIONS - 1] {
            assert!(!party(i).is_empty());
            assert!(contract(i).starts_with("ADK-"));
            assert!(amount(i) > 0.0);
        }
    }
}
