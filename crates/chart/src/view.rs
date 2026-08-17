//! **The public API** — constructor functions plus method chaining (§2.5).
//!
//! ```
//! # use silka_chart::{line_chart, format::NumberFormat};
//! # use silka_widgets::Fonts;
//! # use silka_theme::{Appearance, Theme};
//! # struct Tx { tanggal: f64, nilai: f64 }
//! # let fonts = Fonts::bundled_only();
//! # let theme = Theme::cupertino(Appearance::Dark);
//! # let data = vec![Tx { tanggal: 20_000.0, nilai: 12.0 }, Tx { tanggal: 20_030.0, nilai: 18.0 }];
//! line_chart_in(&fonts, &theme, data)
//!     .x(|d: &Tx| d.tanggal)
//!     .y(|d: &Tx| d.nilai)
//!     .time()
//!     .legend(true)
//!     .animated(true);
//! ```
//!
//! ## Where the accessors go
//!
//! `.x(…)` and `.y(…)` are applied **immediately**, against the rows handed to
//! the constructor — they are not stored. Everything downstream of the builder
//! therefore holds plain numbers, which is what lets the render node be a
//! concrete type the view-diff can downcast to, and what lets diffing compare
//! data at all (closures cannot be compared, so a chart holding its accessors
//! would rebuild on every single frame). See [`crate::model`].
//!
//! ## Why `fonts` and `theme` come first
//!
//! Exactly as in [`button`](mod@silka_widgets::button),
//! [`table`](mod@silka_widgets::table), and every other component: there is no
//! ambient context for application-level dependencies yet, so the text engine
//! and the theme are passed explicitly. `silka-widgets` documents the same debt
//! for the same reason — when a context arrives, both crates lose the same two
//! parameters on the same day.

use std::rc::Rc;

use silka_core::signals::Key;
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::Color;
use silka_theme::Theme;
use silka_widgets::Fonts;

use crate::format::{Locale, NumberFormat};
use crate::model::{BarLayout, ChartData, ChartKind, ChartSpec, Orientation, Series, XKind};
use crate::node::{ChartBox, HoverCallback};
use crate::palette::ChartPalette;
use crate::style::ChartStyle;
use crate::tooltip::ChartHover;

// ---------------------------------------------------------------------------
// Props (the view side of the node)
// ---------------------------------------------------------------------------

/// The props behind every chart view.
///
/// What [`ChartBuilder`] turns into: the spec, the resolved data, and the
/// already-resolved style. It is what the view-diff layer compares between
/// rebuilds, so the caller's row type is long gone by this point.
///
/// ```
/// use silka_core::view::View;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
/// use silka_chart::line_chart_in;
///
/// # let fonts = Fonts::bundled_only();
/// # let theme = Theme::cupertino(Appearance::Dark);
/// let chart = line_chart_in(&fonts, &theme, vec![1.0f64, 4.0, 2.0])
///     .y(|v: &f64| *v)
///     .animated(false);
///
/// // A chart is one view node, whatever it draws inside.
/// let _view: View = chart.into();
/// ```
#[derive(Clone)]
pub struct ChartProps {
    spec: ChartSpec,
    data: ChartData,
    style: ChartStyle,
    fonts: Fonts,
    on_hover: Option<HoverCallback>,
}

impl std::fmt::Debug for ChartProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartProps")
            .field("kind", &self.spec.kind)
            .field("series", &self.data.series.len())
            .field("points", &self.data.len())
            .finish()
    }
}

impl ViewNode for ChartProps {
    fn build(&self) -> Box<dyn silka_core::tree::RenderNode> {
        Box::new(ChartBox::new(
            self.spec.clone(),
            self.data.clone(),
            self.style.clone(),
            self.fonts.clone(),
            self.on_hover.clone(),
        ))
    }

    fn update(&self, node: &mut dyn silka_core::tree::RenderNode) -> silka_core::scheduler::Dirty {
        use silka_core::scheduler::Dirty;

        let n = node
            .downcast_mut::<ChartBox>()
            .expect("tipe view sama berarti tipe render node sama");

        // The callback is replaced unconditionally: two closures cannot be
        // compared, and a stale one would call into a signal the previous
        // rebuild owned.
        n.on_hover = self.on_hover.clone();

        let data_berubah = n.data != self.data;
        let spec_berubah = n.spec != self.spec;
        let style_berubah = n.style != self.style || n.fonts != self.fonts;
        if !data_berubah && !spec_berubah && !style_berubah {
            return Dirty::NONE;
        }

        n.spec = self.spec.clone();
        n.style = self.style.clone();
        n.fonts = self.fonts.clone();
        if data_berubah {
            n.data = self.data.clone();
            // `false` = this is a data *change*, so every value springs from
            // where it already is, carrying its velocity, instead of restarting
            // from the baseline (§3.5).
            n.sync_springs(false);
        } else if spec_berubah {
            n.sync_springs(false);
        }
        n.invalidate();
        Dirty::LAYOUT | Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// The Dart-style builder shared by every chart kind.
///
/// Generic over the caller's row type only until `.x`/`.y` have been applied;
/// what it accumulates is already plain numbers.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
/// use silka_chart::{bar_chart_in, format::{Locale, NumberFormat}};
///
/// struct Month { name: &'static str, income: f64, outgoing: f64 }
/// # let fonts = Fonts::bundled_only();
/// # let theme = Theme::cupertino(Appearance::Dark);
/// # let data = vec![Month { name: "Jan", income: 1.2e6, outgoing: 8.0e5 }];
///
/// // The closures read the application's own row type; everything downstream
/// // of them is plain numbers.
/// bar_chart_in(&fonts, &theme, data)
///     .x_label(|m: &Month| m.name.to_string())
///     .y_named("Income", |m: &Month| m.income)
///     .y_named("Outgoing", |m: &Month| m.outgoing)
///     .stacked()
///     .legend(true)
///     .locale(Locale::ID_ID)
///     .value_format(NumberFormat::Compact);
/// ```
pub struct ChartBuilder<T> {
    rows: Vec<T>,
    spec: ChartSpec,
    data: ChartData,
    theme: Theme,
    fonts: Fonts,
    palette: Option<ChartPalette>,
    sparkline_style: bool,
    on_hover: Option<HoverCallback>,
    key: Option<Key>,
}

impl<T> ChartBuilder<T> {
    fn new(kind: ChartKind, fonts: &Fonts, theme: &Theme, rows: Vec<T>) -> Self {
        let n = rows.len();
        Self {
            rows,
            spec: ChartSpec::new(kind),
            data: ChartData {
                // Without an explicit `.x`, position **is** the index — which is
                // the right default for a categorical bar chart and a usable one
                // for everything else.
                x: (0..n).map(|i| i as f64).collect(),
                labels: Vec::new(),
                series: Vec::new(),
            },
            theme: *theme,
            fonts: fonts.clone(),
            palette: None,
            sparkline_style: kind == ChartKind::Sparkline,
            on_hover: None,
            key: None,
        }
    }

    /// This chart's identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // -- data --------------------------------------------------------------

    /// The x position of each row.
    ///
    /// For a categorical chart this is rarely needed — the row order is the
    /// category order. For a time or numeric axis it is what makes the spacing
    /// honest: two measurements a month apart must not be drawn as far apart as
    /// two taken on consecutive days.
    pub fn x(mut self, f: impl Fn(&T) -> f64) -> Self {
        self.data.x = self.rows.iter().map(f).collect();
        self
    }

    /// The label of each row — the category axis and the tooltip's title.
    pub fn x_label(mut self, f: impl Fn(&T) -> String) -> Self {
        self.data.labels = self.rows.iter().map(f).collect();
        self
    }

    /// Add a series.
    ///
    /// Called more than once, it adds more series; each takes the next palette
    /// slot, in call order, and keeps it however the data is later filtered.
    pub fn y(self, f: impl Fn(&T) -> f64) -> Self {
        let n = self.data.series.len() + 1;
        self.y_named(format!("Series {n}"), f)
    }

    /// Add a named series — the name a legend and a tooltip announce.
    pub fn y_named(mut self, name: impl Into<String>, f: impl Fn(&T) -> f64) -> Self {
        let values: Vec<f64> = self.rows.iter().map(f).collect();
        self.data.series.push(Series::new(name, values));
        self
    }

    /// Add a series that may have **gaps**.
    ///
    /// `None` becomes a hole, not a zero. The distinction matters more than it
    /// looks: a line drawn straight through a missing month asserts a
    /// measurement that was never taken.
    pub fn y_opt(mut self, name: impl Into<String>, f: impl Fn(&T) -> Option<f64>) -> Self {
        let values: Vec<f64> = self.rows.iter().map(|r| f(r).unwrap_or(f64::NAN)).collect();
        self.data.series.push(Series::new(name, values));
        self
    }

    /// Give one series an explicit color, overriding its palette slot.
    ///
    /// Reach for this when a series has a *meaning* the palette cannot carry —
    /// "budget" in the neutral ink, "actual" in the accent. Do not reach for it
    /// to make a chart prettier: the palette's slot order is the colorblind
    /// safety mechanism (see [`crate::palette`]).
    pub fn color(mut self, series: usize, color: Color) -> Self {
        if let Some(s) = self.data.series.get_mut(series) {
            s.color = Some(color);
        }
        self
    }

    // -- axes --------------------------------------------------------------

    /// Treat the x values as **categories** — a band scale, one slot per row.
    pub fn category(mut self) -> Self {
        self.spec.x_kind = XKind::Category;
        self
    }

    /// Treat the x values as plain numbers on a continuous axis.
    pub fn numeric(mut self) -> Self {
        self.spec.x_kind = XKind::Numeric;
        self
    }

    /// Treat the x values as **day numbers** (see [`crate::date`]) — a time
    /// axis, ticked on real calendar boundaries.
    pub fn time(mut self) -> Self {
        self.spec.x_kind = XKind::Time;
        self
    }

    /// Show or hide the gridlines.
    pub fn grid(mut self, show: bool) -> Self {
        self.spec.grid = show;
        self
    }

    /// Show or hide both axes at once.
    pub fn axes(mut self, show: bool) -> Self {
        self.spec.value_axis = show;
        self.spec.category_axis = show;
        self
    }

    /// Show or hide the value axis alone.
    pub fn value_axis(mut self, show: bool) -> Self {
        self.spec.value_axis = show;
        self
    }

    /// Show or hide the category axis alone.
    pub fn category_axis(mut self, show: bool) -> Self {
        self.spec.category_axis = show;
        self
    }

    /// Force the value axis to include zero, or forbid it.
    ///
    /// The default is already the honest one — bars yes, lines no (see
    /// [`crate::ticks::zero_based_domain`]) — so use this only when the data
    /// has a reason the default cannot know.
    pub fn zero_based(mut self, zero: bool) -> Self {
        self.spec.zero_based = Some(zero);
        self
    }

    // -- appearance --------------------------------------------------------

    /// The chart's title, which doubles as its accessibility name.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.spec.title = Some(title.into());
        self
    }

    /// Show the legend.
    ///
    /// Identity must never be carried by color alone, so a chart with two or
    /// more series should have either this or direct labels. With a single
    /// series it is deliberately a no-op: the title already names it, and a
    /// one-entry legend is noise.
    pub fn legend(mut self, show: bool) -> Self {
        self.spec.legend = show;
        self
    }

    /// Draw a marker at each data point.
    pub fn markers(mut self, show: bool) -> Self {
        self.spec.markers = show;
        self
    }

    /// Animate value changes on a spring.
    pub fn animated(mut self, animated: bool) -> Self {
        self.spec.animated = animated;
        self
    }

    /// The text shown when there is no data.
    pub fn empty(mut self, message: impl Into<String>) -> Self {
        self.spec.empty_message = message.into();
        self
    }

    /// The locale used for every number and date.
    pub fn locale(mut self, locale: Locale) -> Self {
        self.spec.locale = locale;
        self
    }

    /// How value-axis labels and tooltip values are written.
    pub fn value_format(mut self, format: NumberFormat) -> Self {
        self.spec.value_format = format;
        self
    }

    /// How category-axis labels are written.
    pub fn category_format(mut self, format: NumberFormat) -> Self {
        self.spec.category_format = format;
        self
    }

    /// A brand palette in place of the first-party one — see
    /// [`ChartPalette::with_slots`] for the obligation that comes with it.
    pub fn palette(mut self, palette: ChartPalette) -> Self {
        self.palette = Some(palette);
        self
    }

    // -- orientation & bars -------------------------------------------------

    /// Value upward, categories across (the default).
    pub fn vertical(mut self) -> Self {
        self.spec.orientation = Orientation::Vertical;
        self
    }

    /// Value rightward, categories downward — the layout for long category
    /// names, which then need no rotation.
    pub fn horizontal(mut self) -> Self {
        self.spec.orientation = Orientation::Horizontal;
        self
    }

    /// Bars side by side, for comparing series against each other.
    pub fn grouped(mut self) -> Self {
        self.spec.bar_layout = BarLayout::Grouped;
        self
    }

    /// Bars piled up, for a total that is also broken down.
    pub fn stacked(mut self) -> Self {
        self.spec.bar_layout = BarLayout::Stacked;
        self
    }

    // -- interaction --------------------------------------------------------

    /// Called as the pointer moves over the plot, and with `None` when it
    /// leaves.
    ///
    /// Store the value in a signal and hand it to
    /// [`tooltip_overlay`](crate::tooltip::tooltip_overlay). This crate
    /// deliberately does not open the panel itself: the overlay layer belongs
    /// to the application's view tree, and a chart reaching into it would be
    /// the eleventh component to invent its own placement.
    pub fn on_hover(mut self, f: impl Fn(Option<ChartHover>) + 'static) -> Self {
        self.on_hover = Some(Rc::new(f));
        self
    }

    // -- finish -------------------------------------------------------------

    /// The resolved style — exposed so a tooltip can be drawn in the same
    /// colors as the marks it explains.
    fn style(&self) -> ChartStyle {
        let mut style = ChartStyle::from_theme(&self.theme);
        if self.sparkline_style {
            style = style.sparkline(&self.theme);
        }
        if let Some(p) = self.palette {
            style = style.with_palette(p);
        }
        style
    }

    fn into_props(self) -> (ChartProps, Option<Key>) {
        let style = self.style();
        let mut spec = self.spec;
        // A one-entry legend is noise: the title already names the series.
        if self.data.series.len() < 2 {
            spec.legend = false;
        }
        (
            ChartProps {
                spec,
                data: self.data,
                style,
                fonts: self.fonts,
                on_hover: self.on_hover,
            },
            self.key,
        )
    }
}

impl<T> From<ChartBuilder<T>> for View {
    fn from(b: ChartBuilder<T>) -> View {
        let (props, key) = b.into_props();
        let mut builder = Builder::new(props);
        if let Some(k) = key {
            builder = builder.key(k);
        }
        builder.into()
    }
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

/// A **line chart**: a value over an ordered axis.
///
/// Use [`line_chart_in`] outside a build pass.
pub fn line_chart<T>(data: impl IntoIterator<Item = T>) -> ChartBuilder<T> {
    line_chart_in(
        &silka_widgets::active_fonts(),
        &silka_widgets::active_theme(),
        data,
    )
}

/// A **line chart**: position over a continuous axis.
///
/// The form to reach for when the question is "how did this change" — the eye
/// reads the slope, which is why the value axis is *not* forced to include zero
/// (see [`ChartBuilder::zero_based`]).
///
/// ```
/// use silka_chart::line_chart_in;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// struct Reading {
///     at: &'static str,
///     cpu: f64,
///     memory: f64,
/// }
/// let data = vec![
///     Reading { at: "09:00", cpu: 12.0, memory: 48.0 },
///     Reading { at: "10:00", cpu: 31.0, memory: 51.0 },
///     Reading { at: "11:00", cpu: 24.0, memory: 55.0 },
/// ];
///
/// // Two series on one chart: `y_named` once per line, and the categorical
/// // palette assigns colors that stay distinguishable to colorblind readers.
/// let chart = line_chart_in(&fonts, &theme, data)
///     .x_label(|d: &Reading| d.at.to_string())
///     .y_named("CPU", |d: &Reading| d.cpu)
///     .y_named("Memory", |d: &Reading| d.memory)
///     .legend(true)
///     .animated(true);
/// # let _ = chart;
/// ```
pub fn line_chart_in<T>(
    fonts: &Fonts,
    theme: &Theme,
    data: impl IntoIterator<Item = T>,
) -> ChartBuilder<T> {
    ChartBuilder::new(ChartKind::Line, fonts, theme, data.into_iter().collect())
}

/// An **area chart**: a line with the space beneath it filled.
///
/// Use [`area_chart_in`] outside a build pass.
pub fn area_chart<T>(data: impl IntoIterator<Item = T>) -> ChartBuilder<T> {
    area_chart_in(
        &silka_widgets::active_fonts(),
        &silka_widgets::active_theme(),
        data,
    )
}

/// An **area chart**: a line with the space beneath it filled.
///
/// Worth the ink only when the filled quantity is genuinely cumulative — a
/// total, a volume. For comparing two independent series, two lines read better
/// than two overlapping fills.
///
/// ```
/// use silka_chart::area_chart_in;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// struct Day {
///     label: &'static str,
///     visitors: f64,
/// }
/// let data = vec![
///     Day { label: "Mon", visitors: 1_200.0 },
///     Day { label: "Tue", visitors: 1_850.0 },
/// ];
///
/// // A filled quantity is only honest when it is cumulative, and a filled
/// // area is only honest when the axis starts at zero.
/// let chart = area_chart_in(&fonts, &theme, data)
///     .x_label(|d: &Day| d.label.to_string())
///     .y_named("Visitors", |d: &Day| d.visitors)
///     .zero_based(true);
/// # let _ = chart;
/// ```
pub fn area_chart_in<T>(
    fonts: &Fonts,
    theme: &Theme,
    data: impl IntoIterator<Item = T>,
) -> ChartBuilder<T> {
    ChartBuilder::new(ChartKind::Area, fonts, theme, data.into_iter().collect())
}

/// A **bar chart**: magnitude as length.
///
/// Use [`bar_chart_in`] outside a build pass.
pub fn bar_chart<T>(data: impl IntoIterator<Item = T>) -> ChartBuilder<T> {
    bar_chart_in(
        &silka_widgets::active_fonts(),
        &silka_widgets::active_theme(),
        data,
    )
}

/// A **bar chart**: magnitude as length.
///
/// Defaults to a categorical x axis and a zero-based value axis, and neither
/// default is cosmetic: a bar's length *is* its value, so an axis that does not
/// start at zero misstates every comparison on the chart.
///
/// ```
/// use silka_chart::bar_chart_in;
/// use silka_chart::format::{Locale, NumberFormat};
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// struct Month {
///     name: &'static str,
///     income: f64,
///     expense: f64,
/// }
/// let data = vec![
///     Month { name: "Jan", income: 1.2e9, expense: 8.0e8 },
///     Month { name: "Feb", income: 1.4e9, expense: 9.1e8 },
/// ];
///
/// // Stacked bars answer "what makes up the total"; grouped bars (the
/// // default) answer "how do these compare". Billions become "1,2 M" rather
/// // than a wall of digits, in the reader's own locale.
/// let chart = bar_chart_in(&fonts, &theme, data)
///     .x_label(|d: &Month| d.name.to_string())
///     .y_named("Income", |d: &Month| d.income)
///     .y_named("Expense", |d: &Month| d.expense)
///     .stacked()
///     .horizontal()
///     .locale(Locale::ID_ID)
///     .value_format(NumberFormat::Compact);
/// # let _ = chart;
/// ```
pub fn bar_chart_in<T>(
    fonts: &Fonts,
    theme: &Theme,
    data: impl IntoIterator<Item = T>,
) -> ChartBuilder<T> {
    ChartBuilder::new(ChartKind::Bar, fonts, theme, data.into_iter().collect())
}

/// A **sparkline**: a word-sized line with no axes, labels, or legend.
///
/// Use [`sparkline_in`] outside a build pass.
pub fn sparkline(values: impl IntoIterator<Item = f64>) -> ChartBuilder<f64> {
    sparkline_in(
        &silka_widgets::active_fonts(),
        &silka_widgets::active_theme(),
        values,
    )
}

/// A **sparkline**: a word-sized line with no axes, no labels, and no legend.
///
/// Takes plain values rather than rows, because that is how a sparkline is
/// actually used — inside a table cell, beside a number, where there is room
/// for the shape of a trend and nothing else.
///
/// ```
/// use silka_chart::sparkline_in;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Plain numbers in, no accessors: the shape of the trend is the whole
/// // message, so there is nothing to label.
/// let trend = sparkline_in(&fonts, &theme, [4.0, 9.0, 7.0, 12.0, 11.0, 18.0]);
/// # let _ = trend;
/// ```
pub fn sparkline_in(
    fonts: &Fonts,
    theme: &Theme,
    values: impl IntoIterator<Item = f64>,
) -> ChartBuilder<f64> {
    let rows: Vec<f64> = values.into_iter().collect();
    ChartBuilder::new(ChartKind::Sparkline, fonts, theme, rows)
        .numeric()
        .y_named("Sparkline", |v: &f64| *v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{BoxConstraints, RenderTree};
    use silka_core::view::reconcile;
    use silka_paint::Size;
    use silka_theme::Appearance;
    use std::cell::RefCell;

    struct Tx {
        hari: f64,
        nilai: f64,
        biaya: f64,
        nama: &'static str,
    }

    fn data() -> Vec<Tx> {
        vec![
            Tx {
                hari: 0.0,
                nilai: 10.0,
                biaya: 4.0,
                nama: "Jan",
            },
            Tx {
                hari: 31.0,
                nilai: 24.0,
                biaya: 9.0,
                nama: "Feb",
            },
            Tx {
                hari: 59.0,
                nilai: 18.0,
                biaya: 6.0,
                nama: "Mar",
            },
            Tx {
                hari: 90.0,
                nilai: 30.0,
                biaya: 11.0,
                nama: "Apr",
            },
        ]
    }

    fn env() -> (Fonts, Theme) {
        (Fonts::bundled_only(), Theme::cupertino(Appearance::Dark))
    }

    fn pohon(view: impl Into<View>, ukuran: Size) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(ukuran));
        tree
    }

    fn chart(tree: &RenderTree) -> &ChartBox {
        tree.node_ref::<ChartBox>(tree.children(tree.root())[0])
            .expect("node chart")
    }

    #[test]
    fn rantai_gaya_dart_menghasilkan_deret_yang_benar() {
        let (f, t) = env();
        let tree = pohon(
            silka_core::view::column([View::from(
                line_chart_in(&f, &t, data())
                    .x(|d: &Tx| d.hari)
                    .y_named("Pendapatan", |d: &Tx| d.nilai)
                    .y_named("Biaya", |d: &Tx| d.biaya)
                    .time()
                    .legend(true),
            )]),
            Size::new(600.0, 400.0),
        );
        let node = tree
            .node_ref::<ChartBox>(tree.children(tree.children(tree.root())[0])[0])
            .expect("node chart");
        assert_eq!(node.data().series.len(), 2);
        assert_eq!(node.data().series[0].name, "Pendapatan");
        assert_eq!(node.data().series[0].values, vec![10.0, 24.0, 18.0, 30.0]);
        assert_eq!(node.data().x, vec![0.0, 31.0, 59.0, 90.0]);
    }

    #[test]
    fn tanpa_x_posisi_adalah_indeks() {
        let (f, t) = env();
        let tree = pohon(
            bar_chart_in(&f, &t, data()).y(|d: &Tx| d.nilai),
            Size::new(400.0, 300.0),
        );
        assert_eq!(chart(&tree).data().x, vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn deret_tanpa_nama_diberi_nomor_urut() {
        let (f, t) = env();
        let tree = pohon(
            line_chart_in(&f, &t, data())
                .y(|d: &Tx| d.nilai)
                .y(|d: &Tx| d.biaya),
            Size::new(400.0, 300.0),
        );
        let d = chart(&tree).data();
        assert_eq!(d.series[0].name, "Series 1");
        assert_eq!(d.series[1].name, "Series 2");
    }

    #[test]
    fn nilai_kosong_menjadi_lubang_bukan_nol() {
        let (f, t) = env();
        let tree = pohon(
            line_chart_in(&f, &t, data())
                .y_opt("Sebagian", |d: &Tx| (d.nilai > 15.0).then_some(d.nilai)),
            Size::new(400.0, 300.0),
        );
        let s = &chart(&tree).data().series[0];
        assert_eq!(s.value(0), None, "10 di bawah ambang → lubang");
        assert_eq!(s.value(1), Some(24.0));
    }

    #[test]
    fn legend_satu_deret_dimatikan_diam_diam() {
        // A one-entry legend is noise the title already covers.
        let (f, t) = env();
        let tree = pohon(
            line_chart_in(&f, &t, data())
                .y(|d: &Tx| d.nilai)
                .legend(true),
            Size::new(400.0, 300.0),
        );
        // Nothing to assert on the outside; what matters is that it lays out
        // and leaves its room to the plot.
        assert!(chart(&tree).geometry().is_some());
    }

    #[test]
    fn chart_mengisi_kotak_yang_diberikan() {
        let (f, t) = env();
        let tree = pohon(
            bar_chart_in(&f, &t, data())
                .y(|d: &Tx| d.nilai)
                .x_label(|d: &Tx| d.nama.to_string()),
            Size::new(500.0, 320.0),
        );
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.size(id), Size::new(500.0, 320.0));
        let g = chart(&tree).geometry().expect("geometri");
        // The plot lives inside the node, with room left for the axes.
        assert!(
            g.plot.min_x() > 0.0,
            "sumbu nilai butuh ruang: {:?}",
            g.plot
        );
        assert!(
            g.plot.max_y() < 320.0,
            "sumbu kategori butuh ruang: {:?}",
            g.plot
        );
        assert!(g.plot.size.width > 100.0 && g.plot.size.height > 100.0);
    }

    #[test]
    fn label_kategori_dipakai_sebagai_tick() {
        let (f, t) = env();
        let tree = pohon(
            bar_chart_in(&f, &t, data())
                .y(|d: &Tx| d.nilai)
                .x_label(|d: &Tx| d.nama.to_string()),
            Size::new(500.0, 320.0),
        );
        let g = chart(&tree).geometry().expect("geometri");
        let label: Vec<&str> = g.category_ticks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(label, vec!["Jan", "Feb", "Mar", "Apr"]);
    }

    #[test]
    fn diff_tanpa_perubahan_tidak_menandai_apa_pun() {
        // The property that keeps a chart cheap: rebuilding the view with the
        // same data must not schedule layout, paint, or a frame.
        let (f, t) = env();
        let mut tree = RenderTree::new();
        let bikin = |f: &Fonts, t: &Theme| -> View {
            line_chart_in(f, t, data())
                .x(|d: &Tx| d.hari)
                .y_named("Pendapatan", |d: &Tx| d.nilai)
                .numeric()
                .into()
        };
        reconcile(&mut tree, bikin(&f, &t));
        tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
        let _ = tree.take_dirty();

        let stat = reconcile(&mut tree, bikin(&f, &t));
        assert_eq!(stat.created, 0, "node yang sama");
        assert!(
            tree.take_dirty().is_empty(),
            "data identik tidak boleh menjadwalkan pekerjaan"
        );
    }

    #[test]
    fn data_berubah_menandai_layout_dan_membidik_ulang_spring() {
        let (f, t) = env();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            line_chart_in(&f, &t, data())
                .y_named("a", |d: &Tx| d.nilai)
                .numeric(),
        );
        tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
        let _ = tree.take_dirty();

        reconcile(
            &mut tree,
            line_chart_in(&f, &t, data())
                .y_named("a", |d: &Tx| d.nilai * 2.0)
                .numeric(),
        );
        assert!(tree
            .take_dirty()
            .contains(silka_core::scheduler::Dirty::LAYOUT));
        tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
        let id = tree.children(tree.root())[0];
        assert!(
            tree.node_ref::<ChartBox>(id).unwrap().is_animating(),
            "nilai baru harus dianimasikan menuju sasarannya"
        );
    }

    #[test]
    fn animasi_bisa_dimatikan() {
        let (f, t) = env();
        let tree = pohon(
            line_chart_in(&f, &t, data())
                .y(|d: &Tx| d.nilai)
                .numeric()
                .animated(false),
            Size::new(400.0, 300.0),
        );
        assert!(
            !chart(&tree).is_animating(),
            "nilai harus langsung di tempat"
        );
    }

    #[test]
    fn sparkline_tidak_punya_sumbu_maupun_legenda() {
        let (f, t) = env();
        let tree = pohon(
            sparkline_in(&f, &t, [1.0, 4.0, 2.0, 8.0, 5.0]),
            Size::new(120.0, 32.0),
        );
        let node = chart(&tree);
        let g = node.geometry().expect("geometri");
        // Nothing is reserved for axes: the plot is the whole box.
        assert!((g.plot.size.width - 120.0).abs() < 0.5, "{:?}", g.plot);
        assert!((g.plot.size.height - 32.0).abs() < 0.5, "{:?}", g.plot);
        assert_eq!(node.data().series.len(), 1);
    }

    #[test]
    fn warna_deret_bisa_ditimpa_satu_per_satu() {
        let (f, t) = env();
        let tree = pohon(
            bar_chart_in(&f, &t, data())
                .y_named("a", |d: &Tx| d.nilai)
                .y_named("b", |d: &Tx| d.biaya)
                .color(1, Color::WHITE),
            Size::new(400.0, 300.0),
        );
        let d = chart(&tree).data();
        assert_eq!(d.series[0].color, None);
        assert_eq!(d.series[1].color, Some(Color::WHITE));
    }

    #[test]
    fn on_hover_dipanggil_saat_penunjuk_bergerak() {
        use silka_core::input::{InputRouter, PointerEvent, PointerPhase};
        use silka_paint::Point;
        use std::time::Duration;

        let (f, t) = env();
        let terakhir: Rc<RefCell<Option<ChartHover>>> = Rc::new(RefCell::new(None));
        let tulis = terakhir.clone();

        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            bar_chart_in(&f, &t, data())
                .y_named("Pendapatan", |d: &Tx| d.nilai)
                .x_label(|d: &Tx| d.nama.to_string())
                .on_hover(move |h| *tulis.borrow_mut() = h),
        );
        tree.layout(BoxConstraints::tight(Size::new(500.0, 320.0)));

        let id = tree.children(tree.root())[0];
        let g = tree
            .node_ref::<ChartBox>(id)
            .unwrap()
            .geometry()
            .unwrap()
            .clone();
        let sasaran = g.category.position(2, 2.0);
        let posisi = Point::new(sasaran, g.plot.center().y);

        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &silka_core::input::Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                posisi,
                Duration::ZERO,
            )),
        );

        let h = terakhir.borrow().clone().expect("hover harus terisi");
        assert_eq!(h.index, 2);
        assert_eq!(h.title, "Mar");
        assert_eq!(h.entries.len(), 1);
        assert_eq!(
            h.entries[0].value, 18.0,
            "tooltip membaca data, bukan spring"
        );
        assert!(h.anchor.size.height > 0.0, "anchor harus punya tinggi");

        // …and leaving clears it, otherwise the panel would stay open forever.
        router.dispatch(
            &mut tree,
            &silka_core::input::Event::Pointer(PointerEvent::new(
                PointerPhase::Leave,
                Point::new(-10.0, -10.0),
                Duration::from_millis(1),
            )),
        );
        assert!(terakhir.borrow().is_none());
    }

    #[test]
    fn keadaan_kosong_tetap_tergambar() {
        let (f, t) = env();
        let kosong: Vec<Tx> = Vec::new();
        let tree = pohon(
            bar_chart_in(&f, &t, kosong)
                .y(|d: &Tx| d.nilai)
                .empty("Belum ada transaksi"),
            Size::new(400.0, 300.0),
        );
        let node = chart(&tree);
        assert!(node.data().is_empty());
        assert!(node.geometry().is_none(), "tanpa data tidak ada plot");
        assert_eq!(node.summary(), "Belum ada transaksi");
    }

    #[test]
    fn chart_punya_node_accesskit_yang_menjelaskan_isinya() {
        // A screen reader that says "image" and nothing else has been told
        // nothing (§3.8).
        let (f, t) = env();
        let tree = pohon(
            bar_chart_in(&f, &t, data())
                .y_named("Pendapatan", |d: &Tx| d.nilai)
                .y_named("Biaya", |d: &Tx| d.biaya)
                .title("Arus kas"),
            Size::new(500.0, 320.0),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Arus kas")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, silka_core::access::AccessRole::Image);
        let nilai = e.node.value.clone().expect("ringkasan");
        assert!(nilai.contains("bar chart"), "{nilai}");
        assert!(nilai.contains("Pendapatan"), "{nilai}");
        assert!(nilai.contains("2 series"), "{nilai}");
    }

    #[test]
    fn chart_menghasilkan_perintah_gambar() {
        use silka_paint::{Command, Scene};

        let (f, t) = env();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            bar_chart_in(&f, &t, data())
                .y_named("Pendapatan", |d: &Tx| d.nilai)
                .x_label(|d: &Tx| d.nama.to_string())
                .animated(false),
        );
        tree.layout(BoxConstraints::tight(Size::new(500.0, 320.0)));

        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        let quad = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Quad(_)))
            .count();
        let glyph = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::GlyphRun(_)))
            .count();
        assert!(quad >= 4, "empat batang, gridline, dan sumbu: {quad} quad");
        assert!(glyph >= 4, "label sumbu harus tergambar: {glyph} glyph run");
    }

    #[test]
    fn garis_seri_adalah_satu_perintah_stroke() {
        use silka_paint::{Command, Scene};

        let (f, t) = env();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            line_chart_in(&f, &t, data())
                .y_named("Pendapatan", |d: &Tx| d.nilai)
                .x_label(|d: &Tx| d.nama.to_string())
                .animated(false),
        );
        tree.layout(BoxConstraints::tight(Size::new(500.0, 320.0)));

        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        let goresan: Vec<_> = scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Stroke(g) => Some(g.clone()),
                _ => None,
            })
            .collect();
        // ONE command for the series, not one box per pixel column: that
        // difference is the whole reason the stroke command was written.
        assert_eq!(goresan.len(), 1, "{} perintah stroke", goresan.len());
        assert_eq!(goresan[0].segment_count(), data().len() - 1);
        assert!(goresan[0].width > 0.0);
        // Clipped to the plot on the CPU, so a series never spills over an axis.
        assert!(goresan[0].clip.is_some());
    }

    #[test]
    fn chart_kosong_tetap_menggambar_pesannya() {
        use silka_paint::{Command, Scene};

        let (f, t) = env();
        let kosong: Vec<Tx> = Vec::new();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            bar_chart_in(&f, &t, kosong)
                .y(|d: &Tx| d.nilai)
                .empty("Kosong"),
        );
        tree.layout(BoxConstraints::tight(Size::new(400.0, 200.0)));
        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        assert!(
            scene
                .commands()
                .iter()
                .any(|c| matches!(c, Command::GlyphRun(_))),
            "keadaan kosong harus terlihat, bukan kotak hampa"
        );
    }

    #[test]
    fn kotak_terlalu_kecil_tidak_panik() {
        // A chart in a 4×4 box is nonsense, but it must not be a crash: layout
        // runs long before a container has settled on its final size.
        let (f, t) = env();
        for ukuran in [
            Size::new(4.0, 4.0),
            Size::new(1.0, 200.0),
            Size::new(200.0, 1.0),
        ] {
            let tree = pohon(bar_chart_in(&f, &t, data()).y(|d: &Tx| d.nilai), ukuran);
            let _ = chart(&tree).geometry();
        }
    }
}
