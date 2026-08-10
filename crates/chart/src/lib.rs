//! # silka-chart
//!
//! The chart library for the framework — **a separate crate, not a separate
//! set of rules.**
//!
//! It lives outside `silka-widgets` for one reason: a chart library is large,
//! and the widget catalogue is the framework's frozen public contract
//! (REKOMENDASI §4 "Kestabilan"). Keeping charts out of it means charts can
//! grow without every application paying for them, and without churn in the one
//! crate that must not churn. What the split explicitly does **not** buy is a
//! private set of conventions. Every contract that binds a widget binds a chart:
//!
//! | Contract | Where it is honoured here |
//! |---|---|
//! | Semantic tokens + dual preset, **no hard-coded colors or sizes** (§2.7) | [`style::ChartStyle`] — every value resolves through the theme |
//! | The paint abstraction; **wgpu is never named** (§3.2) | [`stroke`] turns a polyline into `silka-paint` boxes |
//! | Spring animation, retargetable, reduced-motion aware (§3.5) | data transitions in [`node::ChartBox::advance`], driven by [`advance`] |
//! | An **AccessKit node** as part of the widget contract (§3.8) | [`node::ChartBox::summary`] — a description, not a bare "image" |
//! | The overlay system is built once and ridden by all (`KOMPONEN.md` #3) | [`tooltip`] returns an `OverlayBuilder`; it computes no positions |
//! | Dart-style API: constructor + method chain (§2.5) | [`line_chart`], [`bar_chart`], [`area_chart`], [`sparkline`] |
//!
//! ```
//! # use silka_widgets::Fonts;
//! # use silka_theme::{Appearance, Theme};
//! use silka_chart::{bar_chart, format::{Locale, NumberFormat}};
//!
//! # let fonts = Fonts::bundled_only();
//! # let theme = Theme::cupertino(Appearance::Dark);
//! struct Bulan { nama: &'static str, masuk: f64, keluar: f64 }
//! # let data = vec![Bulan { nama: "Jan", masuk: 1.2e6, keluar: 8.0e5 }];
//!
//! bar_chart(&fonts, &theme, data)
//!     .x_label(|d: &Bulan| d.nama.to_string())
//!     .y_named("Masuk", |d: &Bulan| d.masuk)
//!     .y_named("Keluar", |d: &Bulan| d.keluar)
//!     .stacked()
//!     .legend(true)
//!     .animated(true)
//!     .locale(Locale::ID_ID)
//!     .value_format(NumberFormat::Compact);
//! ```
//!
//! ## What is in the box (v1)
//!
//! Four marks — [`line_chart`], [`area_chart`], [`bar_chart`] (vertical or
//! horizontal, grouped or stacked), and [`sparkline`] — over one shared set of
//! elements: axes with ticks and labels, gridlines, a legend, a hover tooltip
//! on the overlay system, locale-aware number and date formatting, an empty
//! state, and spring transitions when the dataset changes.
//!
//! ## The two decisions worth arguing about
//!
//! **Colors do not come from the theme.** Every *other* color in a chart does,
//! but series colors encode identity rather than role, and a role palette has
//! only one accent. So [`palette`] carries a categorical palette that is
//! validated for colorblind readers **by arithmetic in its own unit tests** —
//! and it is the same under both presets, because CVD safety is a promise to
//! the reader, not a brand decision.
//!
//! **A chart is one render node.** Axis space is circular (the value axis's
//! width depends on labels that depend on ticks that depend on the plot height
//! that depends on the category axis), and box constraints rightly forbid a
//! node from reading its sibling's measurements. Resolving it inside a single
//! node is two passes and a comment; see [`node`].
//!
//! ## Not in v1, and deliberately so
//!
//! Pie and donut charts (a form that is worse than a bar chart at the job it is
//! usually given), scatter and bubble plots (they need the *all-pairs* palette
//! gate, which caps the series count at three — see [`palette`]), zoom and pan,
//! and annotations. Each is an addition to this crate rather than a change to
//! it.
//!
//! ## Acknowledged debt
//!
//! `silka-paint` has no stroke command yet, so a line is rasterised into boxes
//! ([`stroke`]) — the same bargain [`silka_widgets::check_dots`] struck, and it
//! collapses to a single command the day the SDF stroke lands. And the
//! accessibility role is [`Image`](silka_core::access::AccessRole::Image) with
//! a description, because `silka-core`'s role vocabulary has no chart role;
//! adding one touches the platform adapter too, so it is a change to make
//! deliberately rather than in passing.

#![warn(missing_docs)]

pub mod date;
pub mod format;
pub mod model;
pub mod motion;
pub mod node;
pub mod palette;
pub mod scale;
pub mod stroke;
pub mod style;
pub mod ticks;
pub mod tooltip;
pub mod view;

pub use date::Date;
pub use format::{Locale, NumberFormat};
pub use model::{
    BarLayout, ChartData, ChartKind, ChartSpec, Orientation, PlotGeometry, Series, Tick, XKind,
};
pub use motion::{advance, is_animating, settle};
pub use node::ChartBox;
pub use palette::{ChartPalette, CATEGORICAL_LEN};
pub use scale::{BandScale, LinearScale};
pub use style::ChartStyle;
pub use ticks::TimeUnit;
pub use tooltip::{tooltip, tooltip_overlay, ChartHover, HoverEntry};
pub use view::{area_chart, bar_chart, line_chart, sparkline, ChartBuilder, ChartProps};
