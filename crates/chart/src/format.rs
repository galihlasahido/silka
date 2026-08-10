//! **Locale-aware number and date formatting** for axis labels, tooltips, and
//! the legend.
//!
//! A finance chart lives or dies on this. `1500000` on an axis is unreadable in
//! any language; `1.500.000` is money to an Indonesian or German reader and
//! nonsense to an American one, who reads the same string as one and a half.
//! Getting the separators wrong is not a cosmetic bug — it changes the number
//! by a factor of a million.
//!
//! Three things are locale-dependent and all three live in [`Locale`]:
//!
//! 1. **Separators** — which character groups thousands, which one marks the
//!    decimal.
//! 2. **Group sizes** — not everyone groups by three. `Locale::EN_IN` groups
//!    `12,34,567` (lakh/crore), and the mechanism that allows it is
//!    [`Locale::group_sizes`], not a special case.
//! 3. **Word-scale abbreviations and date order** — `1,2 jt` vs `1.2M`,
//!    `10 Agu` vs `Aug 10`.
//!
//! What is deliberately **not** here: a full CLDR implementation, currency
//! rounding rules, plural categories, and timezones. Those belong to an i18n
//! layer the whole framework will need (REKOMENDASI §9.8), and when it arrives
//! this module becomes its caller — the *shape* of the API (a locale value
//! handed to a formatter) is the same one CLDR-backed code would expose.
//!
//! ```
//! use silka_chart::format::{Locale, NumberFormat};
//!
//! let uang = NumberFormat::currency("Rp");
//! assert_eq!(uang.format(1_500_000.0, &Locale::ID_ID), "Rp 1.500.000");
//! assert_eq!(NumberFormat::Compact.format(1_500_000.0, &Locale::EN_US), "1.5M");
//! assert_eq!(NumberFormat::Compact.format(1_500_000.0, &Locale::ID_ID), "1,5 jt");
//! ```

use crate::date::Date;
use crate::ticks::TimeUnit;

/// Where a currency symbol sits relative to the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CurrencyPosition {
    /// `$1,500` — directly in front.
    Prefix,
    /// `Rp 1.500` — in front, with a space.
    PrefixSpaced,
    /// `1.500 €` — after, with a space.
    Suffix,
}

/// How dates are ordered when both day and month appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateOrder {
    /// `10 Aug` — most of the world.
    DayMonth,
    /// `Aug 10` — the United States.
    MonthDay,
}

/// One word-scale abbreviation: the magnitude it starts at and its suffix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactUnit {
    /// The magnitude this suffix represents (1e3, 1e6, …).
    pub magnitude: f64,
    /// The suffix, including any leading space (`"M"`, `" jt"`).
    pub suffix: &'static str,
}

/// Everything about presenting a number or a date that depends on who is
/// reading it.
///
/// A plain value with `'static` contents, like [`Theme`](silka_theme::Theme):
/// switching locale rebuilds it rather than invalidating hidden state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Locale {
    /// The BCP-47 tag, for debugging and for tests to name their case.
    pub tag: &'static str,
    /// The decimal mark.
    pub decimal: char,
    /// The group (thousands) separator.
    pub group: char,
    /// Group sizes from the right; the **last** entry repeats.
    ///
    /// `[3]` gives `1,234,567`; `[3, 2]` gives `12,34,567`.
    pub group_sizes: &'static [u8],
    /// Word-scale abbreviations, largest magnitude first.
    pub compact: &'static [CompactUnit],
    /// Abbreviated month names, January first.
    pub months_short: &'static [&'static str; 12],
    /// Day/month ordering.
    pub date_order: DateOrder,
    /// The letter that marks a quarter (`Q3`, `K3`).
    pub quarter_prefix: &'static str,
    /// Where a currency symbol goes.
    pub currency_position: CurrencyPosition,
}

const COMPACT_EN: &[CompactUnit] = &[
    CompactUnit {
        magnitude: 1e12,
        suffix: "T",
    },
    CompactUnit {
        magnitude: 1e9,
        suffix: "B",
    },
    CompactUnit {
        magnitude: 1e6,
        suffix: "M",
    },
    CompactUnit {
        magnitude: 1e3,
        suffix: "K",
    },
];

const COMPACT_ID: &[CompactUnit] = &[
    CompactUnit {
        magnitude: 1e12,
        suffix: " T",
    },
    CompactUnit {
        magnitude: 1e9,
        suffix: " M",
    },
    CompactUnit {
        magnitude: 1e6,
        suffix: " jt",
    },
    CompactUnit {
        magnitude: 1e3,
        suffix: " rb",
    },
];

const COMPACT_DE: &[CompactUnit] = &[
    CompactUnit {
        magnitude: 1e12,
        suffix: " Bio.",
    },
    CompactUnit {
        magnitude: 1e9,
        suffix: " Mrd.",
    },
    CompactUnit {
        magnitude: 1e6,
        suffix: " Mio.",
    },
    CompactUnit {
        magnitude: 1e3,
        suffix: " Tsd.",
    },
];

// The Indian scale is not the western one shifted: after a thousand it goes to
// a hundred thousand (lakh) and ten million (crore). A formatter that only knew
// K/M/B would abbreviate the digits correctly and still say the wrong word.
const COMPACT_IN: &[CompactUnit] = &[
    CompactUnit {
        magnitude: 1e7,
        suffix: " Cr",
    },
    CompactUnit {
        magnitude: 1e5,
        suffix: " L",
    },
    CompactUnit {
        magnitude: 1e3,
        suffix: "K",
    },
];

const MONTHS_EN: &[&str; 12] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_ID: &[&str; 12] = &[
    "Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des",
];
const MONTHS_DE: &[&str; 12] = &[
    "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
];

impl Locale {
    /// English (United States): `1,234.5`, `Aug 10`.
    pub const EN_US: Locale = Locale {
        tag: "en-US",
        decimal: '.',
        group: ',',
        group_sizes: &[3],
        compact: COMPACT_EN,
        months_short: MONTHS_EN,
        date_order: DateOrder::MonthDay,
        quarter_prefix: "Q",
        currency_position: CurrencyPosition::Prefix,
    };

    /// Indonesian: `1.234,5`, `10 Agu`, `Rp 1.500`.
    pub const ID_ID: Locale = Locale {
        tag: "id-ID",
        decimal: ',',
        group: '.',
        group_sizes: &[3],
        compact: COMPACT_ID,
        months_short: MONTHS_ID,
        date_order: DateOrder::DayMonth,
        quarter_prefix: "K",
        currency_position: CurrencyPosition::PrefixSpaced,
    };

    /// German: `1.234,5`, `10 Aug`, `1.500 €`.
    pub const DE_DE: Locale = Locale {
        tag: "de-DE",
        decimal: ',',
        group: '.',
        group_sizes: &[3],
        compact: COMPACT_DE,
        months_short: MONTHS_DE,
        date_order: DateOrder::DayMonth,
        quarter_prefix: "Q",
        currency_position: CurrencyPosition::Suffix,
    };

    /// English (India): `12,34,567` — the lakh/crore grouping, which exists here
    /// to prove [`Locale::group_sizes`] is a mechanism and not decoration.
    pub const EN_IN: Locale = Locale {
        tag: "en-IN",
        decimal: '.',
        group: ',',
        group_sizes: &[3, 2],
        compact: COMPACT_IN,
        months_short: MONTHS_EN,
        date_order: DateOrder::DayMonth,
        quarter_prefix: "Q",
        currency_position: CurrencyPosition::Prefix,
    };

    /// Every first-party locale — for the gallery's switcher and for tests that
    /// must hold across all of them.
    pub const ALL: [Locale; 4] = [Locale::EN_US, Locale::ID_ID, Locale::DE_DE, Locale::EN_IN];

    /// A number with `decimals` decimal places and grouped integer digits.
    pub fn number(&self, value: f64, decimals: usize) -> String {
        if !value.is_finite() {
            return "—".to_string();
        }
        let decimals = decimals.min(9);
        let negative = value < 0.0;
        // Round first, then split: rounding after splitting turns 9.99 into
        // "9,0" at one decimal.
        let rounded =
            (value.abs() * 10f64.powi(decimals as i32)).round() / 10f64.powi(decimals as i32);
        let text = format!("{rounded:.decimals$}");
        let (whole, fraction) = match text.split_once('.') {
            Some((w, f)) => (w, Some(f)),
            None => (text.as_str(), None),
        };

        let mut out = String::with_capacity(text.len() + whole.len() / 3 + 2);
        if negative && rounded != 0.0 {
            out.push('-');
        }
        out.push_str(&self.group_digits(whole));
        if let Some(f) = fraction {
            out.push(self.decimal);
            out.push_str(f);
        }
        out
    }

    /// A number abbreviated to its word scale: `1.5M`, `1,5 jt`, `12,34 L`.
    pub fn compact(&self, value: f64) -> String {
        if !value.is_finite() {
            return "—".to_string();
        }
        let magnitude = value.abs();
        for unit in self.compact {
            if magnitude >= unit.magnitude {
                let scaled = value / unit.magnitude;
                // One decimal below ten, none above: `9.4M` carries
                // information, `94.3M` is noise at axis size. And no trailing
                // `.0` — `3.0K` reads as a precision the number does not have.
                let decimals =
                    usize::from(scaled.abs() < 10.0 && ((scaled * 10.0).round() as i64) % 10 != 0);
                return format!("{}{}", self.number(scaled, decimals), unit.suffix);
            }
        }
        self.number(
            value,
            usize::from(magnitude < 10.0 && magnitude.fract() != 0.0),
        )
    }

    /// The word-scale unit that suits a magnitude, or `None` below the smallest
    /// one.
    pub fn compact_unit(&self, magnitude: f64) -> Option<CompactUnit> {
        let m = magnitude.abs();
        self.compact.iter().copied().find(|u| m >= u.magnitude)
    }

    /// A number in a **given** word-scale unit.
    ///
    /// This exists because an axis must speak one unit. Letting each label pick
    /// its own produces `0`, `500 jt`, `1 M`, `1,5 M` — four labels in two
    /// different units, which asks the reader to convert in their head at
    /// exactly the moment they are trying to compare magnitudes at a glance.
    /// `decimals` therefore comes from the axis's **step**, not from the value.
    pub fn compact_in(&self, value: f64, unit: Option<CompactUnit>, decimals: usize) -> String {
        match unit {
            Some(u) => format!("{}{}", self.number(value / u.magnitude, decimals), u.suffix),
            None => self.number(value, decimals),
        }
    }

    /// A date at the granularity a time axis is labelled at.
    ///
    /// Month ticks carry their year **only in January**: repeating "2026" on
    /// all twelve marks is noise, dropping it entirely leaves the reader unable
    /// to tell where one year ends.
    pub fn date(&self, days: f64, unit: TimeUnit) -> String {
        if !days.is_finite() {
            return "—".to_string();
        }
        let d = Date::from_days(days.round() as i64);
        let bulan = self.months_short[(d.month.clamp(1, 12) - 1) as usize];
        match unit {
            TimeUnit::Day | TimeUnit::Week => match self.date_order {
                DateOrder::DayMonth => format!("{} {bulan}", d.day),
                DateOrder::MonthDay => format!("{bulan} {}", d.day),
            },
            TimeUnit::Month => {
                if d.month == 1 {
                    format!("{bulan} {}", d.year)
                } else {
                    bulan.to_string()
                }
            }
            TimeUnit::Quarter => format!("{}{} {}", self.quarter_prefix, d.quarter(), d.year),
            TimeUnit::Year => d.year.to_string(),
        }
    }

    /// A full date, for a tooltip where there is room to be unambiguous.
    pub fn date_full(&self, days: f64) -> String {
        if !days.is_finite() {
            return "—".to_string();
        }
        let d = Date::from_days(days.round() as i64);
        let bulan = self.months_short[(d.month.clamp(1, 12) - 1) as usize];
        match self.date_order {
            DateOrder::DayMonth => format!("{} {bulan} {}", d.day, d.year),
            DateOrder::MonthDay => format!("{bulan} {}, {}", d.day, d.year),
        }
    }

    /// Group the digits of an already-formatted, unsigned integer string.
    fn group_digits(&self, digits: &str) -> String {
        let sizes = if self.group_sizes.is_empty() {
            &[3u8][..]
        } else {
            self.group_sizes
        };
        let bytes: Vec<char> = digits.chars().collect();
        let mut out: Vec<char> = Vec::with_capacity(bytes.len() + bytes.len() / 3);
        let mut sisa = bytes.len();
        let mut kelompok = 0usize;
        while sisa > 0 {
            let size = *sizes.get(kelompok).unwrap_or(sizes.last().unwrap()) as usize;
            let size = size.max(1);
            let ambil = size.min(sisa);
            if sisa < bytes.len() {
                out.insert(0, self.group);
            }
            for (i, c) in bytes[sisa - ambil..sisa].iter().enumerate() {
                out.insert(i, *c);
            }
            sisa -= ambil;
            kelompok += 1;
        }
        out.into_iter().collect()
    }
}

impl Default for Locale {
    fn default() -> Self {
        Locale::EN_US
    }
}

/// How one axis (or one series' values) is turned into text.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum NumberFormat {
    /// Decimal places chosen from the tick step — the default, and the right
    /// answer for a value axis whose magnitude the chart only learns at layout
    /// time.
    #[default]
    Auto,
    /// A fixed number of decimal places.
    Fixed(u8),
    /// Word scale: `1.5M`, `1,5 jt`.
    Compact,
    /// A fraction rendered as a percentage (`0.42` → `42%`).
    Percent(u8),
    /// A currency amount; the symbol's position comes from the [`Locale`].
    Currency {
        /// The symbol, e.g. `Rp`, `$`, `€`.
        symbol: String,
        /// Decimal places (0 for rupiah, 2 for dollars).
        decimals: u8,
    },
    /// A date, from a **day number** (see [`crate::date`]).
    Date(TimeUnit),
    /// The category label as-is — the bar axis, whose x values are names.
    Category,
}

impl NumberFormat {
    /// A currency format with no decimals — the common case for a finance
    /// dashboard, where cents are noise at axis size.
    pub fn currency(symbol: impl Into<String>) -> Self {
        NumberFormat::Currency {
            symbol: symbol.into(),
            decimals: 0,
        }
    }

    /// A currency format with decimals.
    pub fn currency_with(symbol: impl Into<String>, decimals: u8) -> Self {
        NumberFormat::Currency {
            symbol: symbol.into(),
            decimals,
        }
    }

    /// Format one value.
    ///
    /// [`NumberFormat::Auto`] has no step to look at here, so it falls back to
    /// "as many decimals as the value needs, up to two". Axis labels go through
    /// [`NumberFormat::format_tick`] instead, which does know the step.
    pub fn format(&self, value: f64, locale: &Locale) -> String {
        match self {
            NumberFormat::Auto => locale.number(value, auto_decimals(value)),
            NumberFormat::Fixed(d) => locale.number(value, *d as usize),
            NumberFormat::Compact => locale.compact(value),
            NumberFormat::Percent(d) => format!("{}%", locale.number(value * 100.0, *d as usize)),
            NumberFormat::Currency { symbol, decimals } => {
                let angka = locale.number(value, *decimals as usize);
                match locale.currency_position {
                    CurrencyPosition::Prefix => format!("{symbol}{angka}"),
                    CurrencyPosition::PrefixSpaced => format!("{symbol} {angka}"),
                    CurrencyPosition::Suffix => format!("{angka} {symbol}"),
                }
            }
            NumberFormat::Date(unit) => locale.date(value, *unit),
            NumberFormat::Category => locale.number(value, auto_decimals(value)),
        }
    }

    /// Format an axis tick, given the distance to the next one.
    ///
    /// The step is what decides the decimals: on a `0, 0.25, 0.5` axis every
    /// label needs two, and on a `0, 25, 50` axis none of them do. Deciding per
    /// *value* instead would print `0`, `0.25`, `0.5` — ragged, and the eye
    /// reads ragged labels as unequal spacing.
    pub fn format_tick(&self, value: f64, step: f64, locale: &Locale) -> String {
        match self {
            NumberFormat::Auto => locale.number(value, decimals_for_step(step)),
            NumberFormat::Percent(_) if step > 0.0 => {
                let d = decimals_for_step(step * 100.0);
                format!("{}%", locale.number(value * 100.0, d))
            }
            NumberFormat::Currency { symbol, decimals } if step > 0.0 => {
                let d = (*decimals as usize).max(decimals_for_step(step));
                let angka = locale.number(value, d);
                match locale.currency_position {
                    CurrencyPosition::Prefix => format!("{symbol}{angka}"),
                    CurrencyPosition::PrefixSpaced => format!("{symbol} {angka}"),
                    CurrencyPosition::Suffix => format!("{angka} {symbol}"),
                }
            }
            other => other.format(value, locale),
        }
    }

    /// Format an axis tick that has to agree with **every other tick on the
    /// same axis**.
    ///
    /// The extra argument is the axis's own extent, and it exists for one case
    /// that [`NumberFormat::format_tick`] cannot get right on its own:
    /// [`NumberFormat::Compact`]. Left to choose per value, it produces `0`,
    /// `500 jt`, `1 M`, `1,5 M` — one axis speaking two units, which asks the
    /// reader to do arithmetic at exactly the moment they were promised a
    /// glance. Given the extent, every label takes the unit the *largest* one
    /// needs, and the step decides how many decimals that unit requires.
    pub fn format_axis(&self, value: f64, step: f64, extent: f64, locale: &Locale) -> String {
        match self {
            NumberFormat::Compact => {
                let unit = locale.compact_unit(extent);
                let dalam_unit = match unit {
                    Some(u) => step / u.magnitude,
                    None => step,
                };
                locale.compact_in(value, unit, decimals_for_step(dalam_unit).min(2))
            }
            other => other.format_tick(value, step, locale),
        }
    }
}

/// How many decimal places a tick step of this size needs.
pub fn decimals_for_step(step: f64) -> usize {
    if !step.is_finite() || step <= 0.0 {
        return 0;
    }
    let exp = step.log10().floor();
    if exp >= 0.0 {
        0
    } else {
        // A 2.5-family step needs one place more than its magnitude suggests.
        let dasar = (-exp) as usize;
        let sisa = step / 10f64.powf(exp);
        if (sisa - sisa.round()).abs() > 1e-9 {
            (dasar + 1).min(9)
        } else {
            dasar.min(9)
        }
    }
}

/// Decimals for a lone value with no step to consult.
fn auto_decimals(value: f64) -> usize {
    let m = value.abs();
    if m >= 100.0 || value.fract() == 0.0 {
        0
    } else if m >= 1.0 {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pemisah_ribuan_berbeda_per_locale() {
        // Getting this wrong does not look slightly off — it changes the number
        // by a factor of a million for half the world's readers.
        assert_eq!(Locale::EN_US.number(1_234_567.0, 0), "1,234,567");
        assert_eq!(Locale::ID_ID.number(1_234_567.0, 0), "1.234.567");
        assert_eq!(Locale::DE_DE.number(1_234_567.0, 0), "1.234.567");
    }

    #[test]
    fn tanda_desimal_ikut_locale() {
        assert_eq!(Locale::EN_US.number(1_234.5, 1), "1,234.5");
        assert_eq!(Locale::ID_ID.number(1_234.5, 1), "1.234,5");
        assert_eq!(Locale::DE_DE.number(0.25, 2), "0,25");
    }

    #[test]
    fn pengelompokan_india_bukan_kelipatan_tiga() {
        // The proof that `group_sizes` is a mechanism: 12,34,567 not 1,234,567.
        assert_eq!(Locale::EN_IN.number(1_234_567.0, 0), "12,34,567");
        assert_eq!(Locale::EN_IN.number(123_456_789.0, 0), "12,34,56,789");
        assert_eq!(Locale::EN_IN.number(999.0, 0), "999");
        assert_eq!(Locale::EN_IN.number(1_000.0, 0), "1,000");
    }

    #[test]
    fn angka_negatif_dan_nol_tidak_kacau() {
        assert_eq!(Locale::EN_US.number(-1_234.0, 0), "-1,234");
        assert_eq!(Locale::ID_ID.number(0.0, 0), "0");
        // −0.4 rounded to zero decimals must not come out as "-0".
        assert_eq!(Locale::EN_US.number(-0.4, 0), "0");
    }

    #[test]
    fn pembulatan_terjadi_sebelum_pemisahan() {
        // Splitting first turns 9.99 into "9,0" — a bug that only shows up on
        // the one axis label a reader is looking at.
        assert_eq!(Locale::EN_US.number(9.99, 1), "10.0");
        assert_eq!(Locale::EN_US.number(999.6, 0), "1,000");
    }

    #[test]
    fn nilai_tidak_hingga_tidak_membocorkan_nan_ke_layar() {
        assert_eq!(Locale::EN_US.number(f64::NAN, 2), "—");
        assert_eq!(Locale::ID_ID.compact(f64::INFINITY), "—");
        assert_eq!(Locale::EN_US.date(f64::NAN, TimeUnit::Day), "—");
    }

    #[test]
    fn skala_kata_mengikuti_bahasa() {
        assert_eq!(Locale::EN_US.compact(1_500_000.0), "1.5M");
        assert_eq!(Locale::ID_ID.compact(1_500_000.0), "1,5 jt");
        assert_eq!(Locale::DE_DE.compact(1_500_000.0), "1,5 Mio.");
        assert_eq!(Locale::EN_US.compact(2_400_000_000.0), "2.4B");
        assert_eq!(Locale::EN_US.compact(-3_000.0), "-3K");
        assert_eq!(Locale::EN_US.compact(750.0), "750");
    }

    #[test]
    fn skala_india_memakai_lakh_dan_crore() {
        // Not the western scale shifted: 1e5 and 1e7 are the real steps.
        assert_eq!(Locale::EN_IN.compact(150_000.0), "1.5 L");
        assert_eq!(Locale::EN_IN.compact(25_000_000.0), "2.5 Cr");
    }

    #[test]
    fn desimal_ringkas_hanya_di_bawah_sepuluh() {
        assert_eq!(Locale::EN_US.compact(9_400_000.0), "9.4M");
        assert_eq!(Locale::EN_US.compact(94_300_000.0), "94M");
    }

    #[test]
    fn urutan_tanggal_mengikuti_locale() {
        let hari = Date::new(2026, 8, 10).to_days() as f64;
        assert_eq!(Locale::EN_US.date(hari, TimeUnit::Day), "Aug 10");
        assert_eq!(Locale::ID_ID.date(hari, TimeUnit::Day), "10 Agu");
        assert_eq!(Locale::DE_DE.date(hari, TimeUnit::Day), "10 Aug");
    }

    #[test]
    fn label_bulan_membawa_tahun_hanya_di_januari() {
        // Twelve repetitions of "2026" is noise; none at all leaves the reader
        // unable to see where the year turns over.
        let jan = Date::new(2026, 1, 1).to_days() as f64;
        let agu = Date::new(2026, 8, 1).to_days() as f64;
        assert_eq!(Locale::ID_ID.date(jan, TimeUnit::Month), "Jan 2026");
        assert_eq!(Locale::ID_ID.date(agu, TimeUnit::Month), "Agu");
    }

    #[test]
    fn kuartal_dan_tahun() {
        let hari = Date::new(2026, 7, 1).to_days() as f64;
        assert_eq!(Locale::EN_US.date(hari, TimeUnit::Quarter), "Q3 2026");
        assert_eq!(Locale::ID_ID.date(hari, TimeUnit::Quarter), "K3 2026");
        assert_eq!(Locale::EN_US.date(hari, TimeUnit::Year), "2026");
    }

    #[test]
    fn tanggal_penuh_untuk_tooltip() {
        let hari = Date::new(2026, 8, 10).to_days() as f64;
        assert_eq!(Locale::ID_ID.date_full(hari), "10 Agu 2026");
        assert_eq!(Locale::EN_US.date_full(hari), "Aug 10, 2026");
    }

    #[test]
    fn posisi_simbol_mata_uang_ikut_locale() {
        let f = NumberFormat::currency("Rp");
        assert_eq!(f.format(1_500_000.0, &Locale::ID_ID), "Rp 1.500.000");
        assert_eq!(
            NumberFormat::currency("$").format(1_500.0, &Locale::EN_US),
            "$1,500"
        );
        assert_eq!(
            NumberFormat::currency("€").format(1_500.0, &Locale::DE_DE),
            "1.500 €"
        );
    }

    #[test]
    fn persen_mengalikan_seratus() {
        assert_eq!(NumberFormat::Percent(0).format(0.42, &Locale::EN_US), "42%");
        assert_eq!(
            NumberFormat::Percent(1).format(0.4237, &Locale::ID_ID),
            "42,4%"
        );
    }

    #[test]
    fn desimal_tick_ditentukan_langkah_bukan_nilai() {
        // On a 0, 0.25, 0.5 axis *every* label needs two decimals — including
        // the zero. Deciding per value gives "0", "0.25", "0.5", which the eye
        // reads as uneven spacing.
        let f = NumberFormat::Auto;
        assert_eq!(f.format_tick(0.0, 0.25, &Locale::EN_US), "0.00");
        assert_eq!(f.format_tick(0.5, 0.25, &Locale::EN_US), "0.50");
        assert_eq!(f.format_tick(50.0, 25.0, &Locale::EN_US), "50");
    }

    #[test]
    fn satu_sumbu_memakai_satu_satuan() {
        // Left to choose per value, a compact axis prints "0 · 500 jt · 1 M ·
        // 1,5 M" — one axis speaking two units, which asks the reader to
        // convert in their head at precisely the moment they were promised a
        // glance. `format_axis` takes the unit from the axis's extent, so every
        // label agrees.
        let f = NumberFormat::Compact;
        let (langkah, rentang) = (5.0e8, 1.5e9);
        let label: Vec<String> = [0.0, 5.0e8, 1.0e9, 1.5e9]
            .iter()
            .map(|v| f.format_axis(*v, langkah, rentang, &Locale::ID_ID))
            .collect();
        // Same unit *and* the same number of decimals: ragged labels read as
        // uneven spacing, which is why the count comes from the step.
        assert_eq!(label, ["0,0 M", "0,5 M", "1,0 M", "1,5 M"]);
        // …and the per-value form is still what a tooltip wants, where a single
        // number stands alone.
        assert_eq!(f.format(5.0e8, &Locale::ID_ID), "500 jt");
    }

    #[test]
    fn satuan_sumbu_mengikuti_besarannya() {
        let f = NumberFormat::Compact;
        assert_eq!(f.format_axis(2.0e6, 1.0e6, 5.0e6, &Locale::EN_US), "2M");
        assert_eq!(
            f.format_axis(2_000.0, 1_000.0, 5_000.0, &Locale::EN_US),
            "2K"
        );
        // Below the smallest unit there is no suffix at all.
        assert_eq!(f.format_axis(20.0, 10.0, 50.0, &Locale::EN_US), "20");
    }

    #[test]
    fn format_lain_tidak_berubah_di_sumbu() {
        // Only `Compact` needs the extent; everything else must behave exactly
        // as `format_tick` does, or an axis would silently change meaning.
        for f in [
            NumberFormat::Auto,
            NumberFormat::Fixed(1),
            NumberFormat::Percent(0),
            NumberFormat::currency("Rp"),
        ] {
            assert_eq!(
                f.format_axis(0.5, 0.25, 1.0, &Locale::EN_US),
                f.format_tick(0.5, 0.25, &Locale::EN_US),
                "{f:?}"
            );
        }
    }

    #[test]
    fn desimal_untuk_langkah() {
        assert_eq!(decimals_for_step(25.0), 0);
        assert_eq!(decimals_for_step(1.0), 0);
        assert_eq!(decimals_for_step(0.5), 1);
        assert_eq!(decimals_for_step(0.25), 2);
        assert_eq!(decimals_for_step(0.1), 1);
        assert_eq!(decimals_for_step(0.0), 0);
        assert_eq!(decimals_for_step(f64::NAN), 0);
    }

    #[test]
    fn setiap_locale_menjawab_seluruh_kosakata() {
        // The same guarantee the theme makes about tokens: no locale may leave
        // a case unanswered, because that case would only surface for the
        // readers who use it.
        let hari = Date::new(2026, 3, 15).to_days() as f64;
        for l in Locale::ALL {
            assert!(!l.number(1_234.5, 1).is_empty(), "{}", l.tag);
            assert!(!l.compact(1_234_567.0).is_empty(), "{}", l.tag);
            for unit in [
                TimeUnit::Day,
                TimeUnit::Week,
                TimeUnit::Month,
                TimeUnit::Quarter,
                TimeUnit::Year,
            ] {
                assert!(!l.date(hari, unit).is_empty(), "{} {}", l.tag, unit.name());
            }
            assert_eq!(l.months_short.len(), 12, "{}", l.tag);
        }
    }
}
