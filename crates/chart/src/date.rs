//! **Civil dates from a day number** — the whole calendar this crate needs, and
//! not one line more.
//!
//! A time axis has to answer three questions: which day is this, when does the
//! next month start, and how many days are in it. Pulling in a date-time crate
//! to answer them would drag a timezone database, a parser, and a leap-second
//! policy into a chart library — dependencies the framework would then own
//! forever (REKOMENDASI §3 keeps external crates to the ones that earn their
//! keep).
//!
//! So the axis speaks **days since 1970-01-01** as a plain number, and this
//! module converts to and from the proleptic Gregorian calendar using Howard
//! Hinnant's `civil_from_days`/`days_from_civil` — branch-free integer
//! arithmetic that is correct for any year, leap years included.
//!
//! What this deliberately does **not** do: timezones, daylight saving, and
//! sub-day resolution. An application whose chart needs those converts to day
//! numbers in its own vocabulary before handing data over — which is also the
//! only place that knows which timezone the reader is in.
//!
//! ```
//! use silka_chart::date::Date;
//!
//! assert_eq!(Date::from_days(0), Date::new(1970, 1, 1));
//! assert_eq!(Date::new(2026, 8, 10).to_days(), 20_675);
//! // Leap years are not a special case here, they fall out of the arithmetic.
//! assert_eq!(Date::new(2024, 2, 29).to_days() + 1, Date::new(2024, 3, 1).to_days());
//! ```

/// A calendar date in the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// The year (may be negative).
    pub year: i32,
    /// The month, 1–12.
    pub month: u32,
    /// The day of the month, 1–31.
    pub day: u32,
}

impl Date {
    /// A date from its parts. The parts are trusted; use [`Date::from_days`]
    /// for anything computed.
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// The date `days` days after 1970-01-01 (negative values go backwards).
    ///
    /// Hinnant's algorithm: shift the epoch to 0000-03-01 so that the leap day
    /// lands at the *end* of the year, which is what makes the era arithmetic
    /// below free of special cases.
    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11], March = 0
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        Self {
            year: (y + i64::from(m <= 2)) as i32,
            month: m as u32,
            day: d as u32,
        }
    }

    /// Days since 1970-01-01.
    pub fn to_days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400; // [0, 399]
        let m = i64::from(self.month);
        let d = i64::from(self.day);
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// The first day of this date's month.
    pub fn start_of_month(self) -> Self {
        Self::new(self.year, self.month, 1)
    }

    /// The first day of this date's year.
    pub fn start_of_year(self) -> Self {
        Self::new(self.year, 1, 1)
    }

    /// The first day of this date's quarter.
    pub fn start_of_quarter(self) -> Self {
        Self::new(self.year, (self.month - 1) / 3 * 3 + 1, 1)
    }

    /// This date's quarter, 1–4.
    pub fn quarter(self) -> u32 {
        (self.month - 1) / 3 + 1
    }

    /// The first day of `months` months later (or earlier, if negative).
    pub fn add_months(self, months: i32) -> Self {
        let total = i64::from(self.year) * 12 + i64::from(self.month) - 1 + i64::from(months);
        let year = total.div_euclid(12) as i32;
        let month = total.rem_euclid(12) as u32 + 1;
        let day = self.day.min(days_in_month(year, month));
        Self::new(year, month, day)
    }

    /// The day of the week, 0 = Monday … 6 = Sunday.
    ///
    /// ISO order rather than the American one, because the only thing this is
    /// used for is snapping a week tick to its start, and half the world starts
    /// its week on Monday.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday, which is index 3 in ISO order — hence the
        // `+ 3` before the modulo.
        (self.to_days() + 3).rem_euclid(7) as u32
    }

    /// The Monday of this date's week.
    pub fn start_of_week(self) -> Self {
        Self::from_days(self.to_days() - i64::from(self.weekday()))
    }
}

/// How many days a month has, leap years included.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

/// True for a Gregorian leap year.
pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_adalah_1970_01_01() {
        assert_eq!(Date::from_days(0), Date::new(1970, 1, 1));
        assert_eq!(Date::new(1970, 1, 1).to_days(), 0);
    }

    #[test]
    fn bolak_balik_konsisten_lintas_abad() {
        // A round trip over a wide span is what catches an era-arithmetic
        // mistake: those are invisible near the epoch and wrong by a day
        // centuries away.
        for hari in [-100_000i64, -1, 0, 1, 10_000, 20_675, 100_000] {
            let d = Date::from_days(hari);
            assert_eq!(d.to_days(), hari, "{d:?}");
        }
    }

    #[test]
    fn tahun_kabisat_bukan_kasus_khusus() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
        assert!(
            !is_leap_year(1900),
            "abad bukan kelipatan 400 bukan kabisat"
        );
        assert!(is_leap_year(2000));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(
            Date::new(2024, 2, 29).to_days() + 1,
            Date::new(2024, 3, 1).to_days()
        );
        assert_eq!(
            Date::new(2023, 2, 28).to_days() + 1,
            Date::new(2023, 3, 1).to_days()
        );
    }

    #[test]
    fn awal_periode_menjepit_ke_batasnya() {
        let d = Date::new(2026, 8, 10);
        assert_eq!(d.start_of_month(), Date::new(2026, 8, 1));
        assert_eq!(d.start_of_year(), Date::new(2026, 1, 1));
        assert_eq!(d.start_of_quarter(), Date::new(2026, 7, 1));
        assert_eq!(d.quarter(), 3);
        assert_eq!(Date::new(2026, 1, 5).quarter(), 1);
        assert_eq!(
            Date::new(2026, 12, 31).start_of_quarter(),
            Date::new(2026, 10, 1)
        );
    }

    #[test]
    fn tambah_bulan_melintasi_tahun_dan_memendekkan_hari() {
        assert_eq!(
            Date::new(2026, 11, 15).add_months(3),
            Date::new(2027, 2, 15)
        );
        assert_eq!(
            Date::new(2026, 2, 15).add_months(-3),
            Date::new(2025, 11, 15)
        );
        // 31 January + 1 month is 28/29 February, not "31 February".
        assert_eq!(Date::new(2026, 1, 31).add_months(1), Date::new(2026, 2, 28));
        assert_eq!(Date::new(2024, 1, 31).add_months(1), Date::new(2024, 2, 29));
    }

    #[test]
    fn hari_dalam_pekan_dan_awal_pekan() {
        // 1970-01-01 was a Thursday → index 3 in ISO order.
        assert_eq!(Date::new(1970, 1, 1).weekday(), 3);
        // 2026-08-10 is a Monday.
        assert_eq!(Date::new(2026, 8, 10).weekday(), 0);
        assert_eq!(
            Date::new(2026, 8, 14).start_of_week(),
            Date::new(2026, 8, 10)
        );
        assert_eq!(
            Date::new(2026, 8, 10).start_of_week(),
            Date::new(2026, 8, 10)
        );
    }
}
