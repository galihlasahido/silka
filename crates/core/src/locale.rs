//! **Who is reading this** — separators, word scale, month and weekday names,
//! date order (REKOMENDASI §9.8).
//!
//! Three components ask the same questions of the same table. A chart axis asks
//! "how do I write 1500000?" and "what is this month called?"; a `calendar`
//! grid asks "what is Wednesday called, and which column does it sit in?"; a
//! `date_picker` asks both. Answering them twice is not a duplication that
//! merely costs lines — it is a duplication that goes **wrong in one place
//! only**, and the place it goes wrong is the one the team writing it cannot
//! read.
//!
//! Getting separators wrong is not cosmetic. `1500000` is unreadable in any
//! language; `1.500.000` is money to an Indonesian or German reader and
//! nonsense to an American one, who reads the same string as one and a half.
//!
//! Five things depend on the reader and all five live in [`Locale`]:
//!
//! 1. **Separators** — which character groups thousands, which one marks the
//!    decimal.
//! 2. **Group sizes** — not everyone groups by three. [`Locale::EN_IN`] groups
//!    `12,34,567` (lakh/crore), and the mechanism that allows it is
//!    [`Locale::group_sizes`], not a special case.
//! 3. **Word-scale abbreviations and date order** — `1,2 jt` vs `1.2M`,
//!    `10 Agu` vs `Aug 10`.
//! 4. **Month and weekday names**, abbreviated and in full.
//! 5. **Which day the week starts on** — [`Locale::first_weekday`]. A calendar
//!    grid with its first column one day out is the single most common
//!    calendar bug, and it is invisible to whoever shares the author's habit.
//!
//! What is deliberately **not** here: a full CLDR implementation, currency
//! rounding rules, plural categories, timezones, and non-Gregorian calendars.
//! Those belong to an i18n layer the whole framework will eventually need, and
//! when it arrives this module becomes its caller — the *shape* of the API (a
//! locale value handed to a formatter) is the same one CLDR-backed code would
//! expose.
//!
//! It lives in `silka-core` beside [`crate::date`] and
//! [`crate::tree::TextDirection`] — the crate that already owns the other half
//! of internationalisation — rather than in either of its callers:
//! `silka-chart` depends on `silka-widgets` and never the other way round, so a
//! calendar in the widget catalogue could not have borrowed the chart's table.
//! `silka_chart::format` re-exports everything here, so nothing that already
//! spoke that path had to change.
//!
//! ```
//! use silka_core::locale::Locale;
//!
//! assert_eq!(Locale::EN_US.number(1_234_567.0, 0), "1,234,567");
//! assert_eq!(Locale::ID_ID.number(1_234_567.0, 0), "1.234.567");
//! assert_eq!(Locale::EN_US.compact(1_500_000.0), "1.5M");
//! assert_eq!(Locale::ID_ID.compact(1_500_000.0), "1,5 jt");
//! ```

use crate::date::{Date, TimeUnit};

/// Where a currency symbol sits relative to the number.
///
/// ```
/// use silka_core::locale::{CurrencyPosition, Locale};
///
/// assert_eq!(Locale::ID_ID.currency_position, CurrencyPosition::PrefixSpaced);
/// assert_eq!(Locale::EN_US.currency_position, CurrencyPosition::Prefix);
/// assert_eq!(Locale::DE_DE.currency_position, CurrencyPosition::Suffix);
/// ```
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
///
/// ```
/// use silka_core::date::{Date, TimeUnit};
/// use silka_core::locale::Locale;
///
/// let days = Date::new(2026, 8, 10).to_days() as f64;
///
/// // The same tick, two reading habits — decided by the locale, not the chart.
/// assert_eq!(Locale::EN_US.date(days, TimeUnit::Day), "Aug 10");
/// assert_eq!(Locale::ID_ID.date(days, TimeUnit::Day), "10 Agu");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateOrder {
    /// `10 Aug` — most of the world.
    DayMonth,
    /// `Aug 10` — the United States.
    MonthDay,
}

/// One word-scale abbreviation: the magnitude it starts at and its suffix.
///
/// Word scale is not a suffix table that can be translated word for word: some
/// locales group by thousands and some do not, so each [`Locale`] carries its
/// own list, largest magnitude first.
///
/// ```
/// use silka_core::locale::Locale;
///
/// assert_eq!(Locale::EN_US.compact(1.5e6), "1.5M");
/// assert_eq!(Locale::ID_ID.compact(1.5e6), "1,5 jt");
/// ```
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
///
/// ```
/// use silka_core::locale::Locale;
///
/// // Separators are a locale property, never a constant in widget code.
/// assert_eq!(Locale::EN_US.number(1234567.0, 0), "1,234,567");
/// assert_eq!(Locale::ID_ID.number(1234567.0, 0), "1.234.567");
///
/// // Group sizes are not always three: the last entry repeats.
/// assert_eq!(Locale::EN_IN.number(1234567.0, 0), "12,34,567");
///
/// // Every built-in locale is available for a cross-locale test sweep.
/// assert_eq!(Locale::ALL.len(), 4);
/// ```
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
    /// Full month names, January first — a calendar's own header.
    pub months_long: &'static [&'static str; 12],
    /// Abbreviated weekday names in **ISO order**, Monday first.
    ///
    /// ISO and not the reader's own order, deliberately: the storage order and
    /// the display order are two different things, and conflating them is how a
    /// grid ends up labelling its columns correctly while filling them one day
    /// out. [`Locale::weekday_columns`] is the one that reorders.
    pub weekdays_short: &'static [&'static str; 7],
    /// One- or two-letter weekday names in ISO order, Monday first — the
    /// column headings of a month grid, where there is no room for more.
    pub weekdays_narrow: &'static [&'static str; 7],
    /// The day the week starts on, ISO-indexed (0 = Monday, 6 = Sunday).
    pub first_weekday: u32,
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

const MONTHS_LONG_EN: &[&str; 12] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_LONG_ID: &[&str; 12] = &[
    "Januari",
    "Februari",
    "Maret",
    "April",
    "Mei",
    "Juni",
    "Juli",
    "Agustus",
    "September",
    "Oktober",
    "November",
    "Desember",
];
const MONTHS_LONG_DE: &[&str; 12] = &[
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

// Every weekday table below is Monday-first (ISO). Which of them a grid puts in
// its leftmost column is `first_weekday`'s business, never the table's.
const WEEKDAYS_EN: &[&str; 7] = &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_NARROW_EN: &[&str; 7] = &["M", "T", "W", "T", "F", "S", "S"];
const WEEKDAYS_ID: &[&str; 7] = &["Sen", "Sel", "Rab", "Kam", "Jum", "Sab", "Min"];
const WEEKDAYS_NARROW_ID: &[&str; 7] = &["S", "S", "R", "K", "J", "S", "M"];
const WEEKDAYS_DE: &[&str; 7] = &["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const WEEKDAYS_NARROW_DE: &[&str; 7] = &["M", "D", "M", "D", "F", "S", "S"];

impl Locale {
    /// English (United States): `1,234.5`, `Aug 10`, weeks starting Sunday.
    pub const EN_US: Locale = Locale {
        tag: "en-US",
        decimal: '.',
        group: ',',
        group_sizes: &[3],
        compact: COMPACT_EN,
        months_short: MONTHS_EN,
        months_long: MONTHS_LONG_EN,
        weekdays_short: WEEKDAYS_EN,
        weekdays_narrow: WEEKDAYS_NARROW_EN,
        // Sunday, which is exactly the difference a European reader never
        // notices in their own calendar and an American reader never notices in
        // theirs.
        first_weekday: 6,
        date_order: DateOrder::MonthDay,
        quarter_prefix: "Q",
        currency_position: CurrencyPosition::Prefix,
    };

    /// Indonesian: `1.234,5`, `10 Agu`, `Rp 1.500`, weeks starting Monday.
    pub const ID_ID: Locale = Locale {
        tag: "id-ID",
        decimal: ',',
        group: '.',
        group_sizes: &[3],
        compact: COMPACT_ID,
        months_short: MONTHS_ID,
        months_long: MONTHS_LONG_ID,
        weekdays_short: WEEKDAYS_ID,
        weekdays_narrow: WEEKDAYS_NARROW_ID,
        first_weekday: 0,
        date_order: DateOrder::DayMonth,
        quarter_prefix: "K",
        currency_position: CurrencyPosition::PrefixSpaced,
    };

    /// German: `1.234,5`, `10 Aug`, `1.500 €`, weeks starting Monday.
    pub const DE_DE: Locale = Locale {
        tag: "de-DE",
        decimal: ',',
        group: '.',
        group_sizes: &[3],
        compact: COMPACT_DE,
        months_short: MONTHS_DE,
        months_long: MONTHS_LONG_DE,
        weekdays_short: WEEKDAYS_DE,
        weekdays_narrow: WEEKDAYS_NARROW_DE,
        first_weekday: 0,
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
        months_long: MONTHS_LONG_EN,
        weekdays_short: WEEKDAYS_EN,
        weekdays_narrow: WEEKDAYS_NARROW_EN,
        first_weekday: 6,
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
        let bulan = self.month_short(d.month);
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
        let bulan = self.month_short(d.month);
        match self.date_order {
            DateOrder::DayMonth => format!("{} {bulan} {}", d.day, d.year),
            DateOrder::MonthDay => format!("{bulan} {}, {}", d.day, d.year),
        }
    }

    // -- calendar vocabulary --------------------------------------------------

    /// The abbreviated name of month `month` (1–12), clamped rather than
    /// panicking on a computed value.
    pub fn month_short(&self, month: u32) -> &'static str {
        self.months_short[(month.clamp(1, 12) - 1) as usize]
    }

    /// The full name of month `month` (1–12) — a calendar's own heading.
    pub fn month_long(&self, month: u32) -> &'static str {
        self.months_long[(month.clamp(1, 12) - 1) as usize]
    }

    /// The abbreviated name of an **ISO** weekday index (0 = Monday).
    pub fn weekday_short(&self, iso: u32) -> &'static str {
        self.weekdays_short[(iso % 7) as usize]
    }

    /// The narrow name of an **ISO** weekday index (0 = Monday).
    pub fn weekday_narrow(&self, iso: u32) -> &'static str {
        self.weekdays_narrow[(iso % 7) as usize]
    }

    /// The seven narrow weekday names **in column order** for a month grid.
    ///
    /// This is the one function that knows about [`Locale::first_weekday`], and
    /// it is why the tables above stay ISO-ordered: a grid that reordered its
    /// headings by hand while filling its cells from
    /// [`Date::column_from`](crate::date::Date::column_from) would look right
    /// for the author's own locale and be one column out for everyone else.
    ///
    /// ```
    /// use silka_core::locale::Locale;
    ///
    /// // Indonesian weeks start on Monday…
    /// assert_eq!(Locale::ID_ID.weekday_columns()[0], "S"); // Senin
    /// // …American ones on Sunday, and the whole row rotates with it.
    /// assert_eq!(Locale::EN_US.weekday_columns()[0], "S"); // Sunday
    /// assert_eq!(Locale::EN_US.weekday_columns()[1], "M"); // Monday
    /// ```
    pub fn weekday_columns(&self) -> [&'static str; 7] {
        let mut out = [""; 7];
        for (column, slot) in out.iter_mut().enumerate() {
            *slot = self.weekday_narrow((self.first_weekday + column as u32) % 7);
        }
        out
    }

    /// The seven abbreviated weekday names in column order — the accessible
    /// name behind each narrow heading, which on its own says nothing ("T"
    /// could be Tuesday or Thursday).
    pub fn weekday_names(&self) -> [&'static str; 7] {
        let mut out = [""; 7];
        for (column, slot) in out.iter_mut().enumerate() {
            *slot = self.weekday_short((self.first_weekday + column as u32) % 7);
        }
        out
    }

    /// `August 2026` — the heading over a month grid.
    pub fn month_year(&self, date: Date) -> String {
        format!("{} {}", self.month_long(date.month), date.year)
    }

    /// A date spelled out the way a screen reader should hear it, and the way a
    /// date cell's accessible name reads: `10 August 2026` / `August 10, 2026`.
    ///
    /// Deliberately **not** the same string as [`Locale::date_full`]: that one
    /// abbreviates for a tooltip that has to fit, this one is spoken.
    ///
    /// ```
    /// use silka_core::date::Date;
    /// use silka_core::locale::Locale;
    ///
    /// assert_eq!(Locale::ID_ID.date_long(Date::new(2026, 8, 10)), "10 Agustus 2026");
    /// assert_eq!(Locale::EN_US.date_long(Date::new(2026, 8, 10)), "August 10, 2026");
    /// ```
    pub fn date_long(&self, date: Date) -> String {
        let bulan = self.month_long(date.month);
        match self.date_order {
            DateOrder::DayMonth => format!("{} {bulan} {}", date.day, date.year),
            DateOrder::MonthDay => format!("{bulan} {}, {}", date.day, date.year),
        }
    }

    /// A date written in digits only, in this locale's order — what a date
    /// field shows and what it parses back (see [`Locale::parse_numeric`]).
    ///
    /// ```
    /// use silka_core::date::Date;
    /// use silka_core::locale::Locale;
    ///
    /// assert_eq!(Locale::ID_ID.numeric(Date::new(2026, 8, 3)), "03/08/2026");
    /// assert_eq!(Locale::EN_US.numeric(Date::new(2026, 8, 3)), "08/03/2026");
    /// ```
    pub fn numeric(&self, date: Date) -> String {
        match self.date_order {
            DateOrder::DayMonth => format!("{:02}/{:02}/{}", date.day, date.month, date.year),
            DateOrder::MonthDay => format!("{:02}/{:02}/{}", date.month, date.day, date.year),
        }
    }

    /// Read back what [`Locale::numeric`] wrote.
    ///
    /// Strict on purpose: a field that guessed would silently turn `03/08` into
    /// the wrong one of two real dates, and the reader has no way of noticing.
    /// Any separator in `./-` is accepted, because that much really is a typing
    /// habit rather than a meaning.
    ///
    /// ```
    /// use silka_core::date::Date;
    /// use silka_core::locale::Locale;
    ///
    /// assert_eq!(Locale::ID_ID.parse_numeric("3/8/2026"), Some(Date::new(2026, 8, 3)));
    /// assert_eq!(Locale::EN_US.parse_numeric("3/8/2026"), Some(Date::new(2026, 3, 8)));
    /// // Not a date: refused rather than rounded into one.
    /// assert_eq!(Locale::EN_US.parse_numeric("13/40/2026"), None);
    /// assert_eq!(Locale::EN_US.parse_numeric(""), None);
    /// ```
    pub fn parse_numeric(&self, text: &str) -> Option<Date> {
        let bagian: Vec<&str> = text
            .split(|c: char| c == '/' || c == '.' || c == '-')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if bagian.len() != 3 {
            return None;
        }
        let a: u32 = bagian[0].parse().ok()?;
        let b: u32 = bagian[1].parse().ok()?;
        let year: i32 = bagian[2].parse().ok()?;
        let (day, month) = match self.date_order {
            DateOrder::DayMonth => (a, b),
            DateOrder::MonthDay => (b, a),
        };
        if !(1..=12).contains(&month) || day < 1 || day > crate::date::days_in_month(year, month) {
            return None;
        }
        Some(Date::new(year, month, day))
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
    fn kolom_pekan_berputar_mengikuti_hari_pertama() {
        // The bug this exists to prevent: a grid whose headings say Monday
        // first while its cells are filled Sunday first. Both sides go through
        // `first_weekday`, so they cannot disagree.
        assert_eq!(Locale::ID_ID.first_weekday, 0);
        assert_eq!(Locale::EN_US.first_weekday, 6);
        assert_eq!(
            Locale::ID_ID.weekday_names(),
            ["Sen", "Sel", "Rab", "Kam", "Jum", "Sab", "Min"]
        );
        assert_eq!(
            Locale::EN_US.weekday_names(),
            ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        );
        // …and a Monday really does land in the column the headings promise.
        let senin = Date::new(2026, 8, 10);
        assert_eq!(
            Locale::ID_ID.weekday_names()[senin.column_from(Locale::ID_ID.first_weekday) as usize],
            "Sen"
        );
        assert_eq!(
            Locale::EN_US.weekday_names()[senin.column_from(Locale::EN_US.first_weekday) as usize],
            "Mon"
        );
    }

    #[test]
    fn nama_bulan_panjang_untuk_judul_kalender() {
        assert_eq!(
            Locale::ID_ID.month_year(Date::new(2026, 8, 1)),
            "Agustus 2026"
        );
        assert_eq!(Locale::DE_DE.month_year(Date::new(2026, 3, 1)), "März 2026");
        // Clamped rather than panicking: the month may be computed.
        assert_eq!(Locale::EN_US.month_long(0), "January");
        assert_eq!(Locale::EN_US.month_long(99), "December");
    }

    #[test]
    fn tanggal_lisan_berbeda_dari_tanggal_tooltip() {
        // One abbreviates because it has to fit, the other is spoken aloud.
        let d = Date::new(2026, 8, 10);
        assert_eq!(Locale::ID_ID.date_long(d), "10 Agustus 2026");
        assert_eq!(Locale::EN_US.date_long(d), "August 10, 2026");
        assert_ne!(
            Locale::EN_US.date_long(d),
            Locale::EN_US.date_full(d.to_days() as f64)
        );
    }

    #[test]
    fn tanggal_numerik_bolak_balik() {
        for l in Locale::ALL {
            for d in [
                Date::new(2026, 8, 3),
                Date::new(2024, 2, 29),
                Date::new(1999, 12, 31),
            ] {
                assert_eq!(l.parse_numeric(&l.numeric(d)), Some(d), "{}", l.tag);
            }
        }
    }

    #[test]
    fn tanggal_numerik_menolak_yang_bukan_tanggal() {
        let l = Locale::EN_US;
        assert_eq!(l.parse_numeric("13/40/2026"), None);
        assert_eq!(l.parse_numeric("2026"), None);
        assert_eq!(l.parse_numeric(""), None);
        assert_eq!(l.parse_numeric("x/y/z"), None);
        // 29 February 2023 does not exist, and guessing 1 March would be worse
        // than refusing.
        assert_eq!(l.parse_numeric("02/29/2023"), None);
        assert_eq!(l.parse_numeric("02/29/2024"), Some(Date::new(2024, 2, 29)));
    }

    #[test]
    fn urutan_hari_bulan_benar_benar_bertukar() {
        // The same three digits, two different real dates. A field that guessed
        // would be wrong half the time and never say so.
        assert_eq!(
            Locale::ID_ID.parse_numeric("3/8/2026"),
            Some(Date::new(2026, 8, 3))
        );
        assert_eq!(
            Locale::EN_US.parse_numeric("3/8/2026"),
            Some(Date::new(2026, 3, 8))
        );
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
            assert_eq!(l.months_long.len(), 12, "{}", l.tag);
            assert_eq!(l.weekdays_short.len(), 7, "{}", l.tag);
            assert_eq!(l.weekdays_narrow.len(), 7, "{}", l.tag);
            assert!(l.first_weekday < 7, "{}", l.tag);
            // The seven columns are the seven days, never six with one twice.
            let mut kolom: Vec<&str> = l.weekday_names().to_vec();
            kolom.sort_unstable();
            kolom.dedup();
            assert_eq!(kolom.len(), 7, "{}", l.tag);
        }
    }
}
