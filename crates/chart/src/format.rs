//! **Locale-aware number and date formatting** for axis labels, tooltips, and
//! the legend.
//!
//! A finance chart lives or dies on this. `1500000` on an axis is unreadable in
//! any language; `1.500.000` is money to an Indonesian or German reader and
//! nonsense to an American one, who reads the same string as one and a half.
//! Getting the separators wrong is not a cosmetic bug — it changes the number
//! by a factor of a million.
//!
//! The table of *who is reading this* — separators, group sizes, word scale,
//! month and weekday names, date order — is [`silka_core::locale::Locale`], and
//! this module re-exports it. It used to live here, and it moved for the same
//! reason [`crate::date`] did: a `calendar` and a `date_picker` in
//! `silka-widgets` ask the very same questions, and `silka-chart` depends on
//! `silka-widgets` rather than the other way round, so the widget catalogue
//! could not have borrowed a copy that lived in this crate. A second table
//! would not merely have cost lines — it would have gone wrong in one of its
//! two homes, and the home it went wrong in is the one its authors cannot read.
//!
//! What stays here is [`NumberFormat`]: *how one axis presents its values*,
//! which is a chart's own decision. It is the caller of the locale, not part of
//! it — and the reason [`NumberFormat::format_axis`] exists is a problem no
//! locale table has: every tick on one axis has to agree with every other.
//!
//! ```
//! use silka_chart::format::{Locale, NumberFormat};
//!
//! let uang = NumberFormat::currency("Rp");
//! assert_eq!(uang.format(1_500_000.0, &Locale::ID_ID), "Rp 1.500.000");
//! assert_eq!(NumberFormat::Compact.format(1_500_000.0, &Locale::EN_US), "1.5M");
//! assert_eq!(NumberFormat::Compact.format(1_500_000.0, &Locale::ID_ID), "1,5 jt");
//! ```

use crate::ticks::TimeUnit;

pub use silka_core::locale::{CompactUnit, CurrencyPosition, DateOrder, Locale};

/// How one axis (or one series' values) is turned into text.
///
/// ```
/// use silka_chart::format::{Locale, NumberFormat};
///
/// let id = Locale::ID_ID;
///
/// assert_eq!(NumberFormat::Fixed(2).format(3.14159, &id), "3,14");
/// assert_eq!(NumberFormat::Percent(0).format(0.42, &id), "42%");
/// assert_eq!(NumberFormat::Compact.format(1.2e9, &id), "1,2 M");
/// assert_eq!(NumberFormat::currency("Rp").format(8e5, &id), "Rp 800.000");
///
/// // `Auto` is the default because a value axis only learns its magnitude at
/// // layout time: the decimals come from the tick step.
/// assert_eq!(NumberFormat::default(), NumberFormat::Auto);
/// assert_eq!(NumberFormat::Auto.format_tick(0.25, 0.25, &id), "0,25");
/// assert_eq!(NumberFormat::Auto.format_tick(2.0, 1.0, &id), "2");
/// ```
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
    fn tanggal_di_sumbu_tetap_lewat_locale() {
        // The chart decides *that* a tick is a date; what it looks like is the
        // locale's business, and this is the seam between the two.
        let hari = crate::date::Date::new(2026, 8, 10).to_days() as f64;
        let f = NumberFormat::Date(TimeUnit::Day);
        assert_eq!(f.format(hari, &Locale::EN_US), "Aug 10");
        assert_eq!(f.format(hari, &Locale::ID_ID), "10 Agu");
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
}
