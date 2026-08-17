//! **Civil dates from a day number** — re-exported from [`silka_core::date`].
//!
//! A time axis has to answer three questions: which day is this, when does the
//! next month start, and how many days are in it. Pulling in a date-time crate
//! to answer them would drag a timezone database, a parser, and a leap-second
//! policy into a chart library — dependencies the framework would then own
//! forever (REKOMENDASI §3 keeps external crates to the ones that earn their
//! keep). So the axis speaks **days since 1970-01-01** as a plain number and
//! converts with Howard Hinnant's branch-free era arithmetic.
//!
//! The arithmetic itself used to live here, and it moved: a `calendar` grid in
//! `silka-widgets` asks the very same three questions, and `silka-chart`
//! depends on `silka-widgets` rather than the other way round, so the widget
//! catalogue could not have borrowed a copy that lived in this crate. It now
//! sits in `silka-core` — the crate that already owns the other half of
//! internationalisation ([`silka_core::tree::TextDirection`]) — and this module
//! is the re-export, so every path that already said `silka_chart::date::Date`
//! still resolves to exactly the same type.
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

pub use silka_core::date::{days_in_month, is_leap_year, Date, TimeUnit};
