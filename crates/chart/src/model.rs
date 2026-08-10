//! **The data model and the plot geometry** — everything a chart knows before
//! a single pixel is involved.
//!
//! This module is where the crate's testability lives. A chart is a long chain
//! of decisions — which domain, which ticks, where does the bar start, which
//! point is under the pointer — and every one of them is arithmetic. Keeping
//! that arithmetic in plain values, with no theme and no render node in sight,
//! is what lets the unit tests assert on *positions* rather than on pixels
//! (REKOMENDASI §9.5: "unit tests for non-visual logic").
//!
//! ## The generic-to-concrete boundary
//!
//! The public builders are generic over the caller's row type (`.x(|d| …)`,
//! `.y(|d| …)`), but [`ChartData`] is not. That is deliberate: the accessors
//! are applied **eagerly**, at view-build time, so what reaches the render tree
//! is plain numbers. Three consequences follow, and all three are the reason
//! for the design:
//!
//! 1. The render node is a concrete type, so view-diff can downcast to it
//!    (the [`ViewNode`](silka_core::view::ViewNode) contract).
//! 2. Diffing compares `Vec<f64>`, not closures — which cannot be compared at
//!    all, and would force every chart to rebuild every frame.
//! 3. The application's row type never has to be `'static` or `Clone`.
//!
//! ## Orientation, without writing everything twice
//!
//! A horizontal bar chart is not a second chart type; it is the same chart with
//! its two axes swapped. So the geometry names its scales by **job** rather
//! than by screen axis: [`PlotGeometry::value`] maps data magnitude,
//! [`PlotGeometry::category`] maps position along the categories, and
//! [`Orientation`] decides which of the two ends up horizontal. Every
//! downstream calculation goes through [`PlotGeometry::point`] and therefore
//! gets both orientations right for free.
//!
//! ```
//! use silka_chart::model::{ChartData, ChartKind, ChartSpec, Orientation, PlotGeometry, Series};
//! use silka_paint::{Point, Rect};
//!
//! // The data reaching the render tree is plain numbers — the caller's
//! // accessors were applied eagerly, at view-build time.
//! let data = ChartData {
//!     x: vec![0.0, 1.0, 2.0],
//!     labels: vec!["Jan".into(), "Feb".into(), "Mar".into()],
//!     series: vec![Series::new("Income", vec![10.0, 30.0, 20.0])],
//! };
//! assert_eq!(data.len(), 3);
//! assert!(!data.is_empty());
//!
//! // A bar chart's value axis includes zero; a line chart's does not. That is
//! // not cosmetic — a bar's *length* is its value.
//! let spec = ChartSpec::new(ChartKind::Bar);
//! assert!(spec.is_zero_based());
//! assert!(!ChartSpec::new(ChartKind::Line).is_zero_based());
//!
//! // Geometry is a pure function of (rect, spec, data), so positions can be
//! // asserted on without a window or a GPU anywhere in sight.
//! let plot = Rect::new(0.0, 0.0, 300.0, 100.0);
//! let g = PlotGeometry::build(plot, &spec, &data);
//! assert_eq!(g.orientation, Orientation::Vertical);
//!
//! // In a vertical chart the largest value sits highest — screen y is
//! // inverted, and this is the one place that knows it.
//! let tallest = g.point(1, 1.0, 30.0);
//! let shortest = g.point(0, 0.0, 10.0);
//! assert!(tallest.y < shortest.y);
//! assert!(tallest.x > shortest.x);
//!
//! // Hovering anywhere in a category's column finds that point, including the
//! // empty space above a short bar.
//! assert_eq!(g.index_at(Point::new(tallest.x, 4.0), &data), Some(1));
//!
//! // And a horizontal chart is the same chart with its axes swapped — no
//! // second code path, so both orientations are right for free.
//! let mut sideways = spec.clone();
//! sideways.orientation = Orientation::Horizontal;
//! let h = PlotGeometry::build(plot, &sideways, &data);
//! let long = h.point(1, 1.0, 30.0);
//! let short = h.point(0, 0.0, 10.0);
//! assert!(long.x > short.x);
//! assert!(long.y > short.y);
//! ```

use silka_paint::{Point, Rect, Size};

use crate::format::{Locale, NumberFormat};
use crate::scale::{BandScale, LinearScale};
use crate::ticks::{self, TimeUnit, MIN_STACKED_TICK_SPACING, MIN_TICK_SPACING};

// ---------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------

/// Which chart is being drawn.
///
/// ```
/// use silka_chart::model::ChartKind;
///
/// // Bars lay out on a band scale and their value axis must include zero —
/// // a bar chart truncated at the bottom lies about its proportions.
/// assert!(ChartKind::Bar.is_bar());
/// assert!(!ChartKind::Bar.is_line());
///
/// // The line family shares one rasterisation path.
/// assert!(ChartKind::Line.is_line());
/// assert!(ChartKind::Area.is_line());
/// assert!(ChartKind::Sparkline.is_line());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// A line through the data points.
    Line,
    /// A line with the space beneath it filled.
    Area,
    /// Bars from the baseline.
    Bar,
    /// A word-sized line with no axes, no labels, and no legend.
    Sparkline,
}

impl ChartKind {
    /// True when the marks are bars — the kinds that lay out on a band scale
    /// and whose value axis must include zero.
    pub fn is_bar(self) -> bool {
        matches!(self, ChartKind::Bar)
    }

    /// True when this kind draws a connected line.
    pub fn is_line(self) -> bool {
        matches!(
            self,
            ChartKind::Line | ChartKind::Area | ChartKind::Sparkline
        )
    }
}

/// How several bar series share one category.
///
/// ```
/// use silka_chart::model::{BarLayout, ChartKind, ChartSpec};
///
/// // Grouped is the default: comparing series against each other is the
/// // commoner question than breaking a total down.
/// assert_eq!(BarLayout::default(), BarLayout::Grouped);
///
/// let mut spec = ChartSpec::new(ChartKind::Bar);
/// assert!(!spec.is_stacked());
/// spec.bar_layout = BarLayout::Stacked;
/// assert!(spec.is_stacked());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BarLayout {
    /// Side by side — for comparing series against each other.
    #[default]
    Grouped,
    /// Piled up — for a total that is also broken down.
    Stacked,
}

/// Which way the value axis runs.
///
/// ```
/// use silka_chart::model::Orientation;
///
/// assert_eq!(Orientation::default(), Orientation::Vertical);
/// ```
///
/// Reach for [`Orientation::Horizontal`] when the category names are long:
/// horizontal labels never need rotating, and a rotated label is the single
/// most common way a chart becomes unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Orientation {
    /// Value upward, categories across. The default for everything.
    #[default]
    Vertical,
    /// Value rightward, categories downward — the layout to reach for when the
    /// category names are long, because horizontal labels never need rotating.
    Horizontal,
}

/// What the x values mean, which decides how they are ticked and labelled.
///
/// ```
/// use silka_chart::model::XKind;
///
/// // Names have no "between": one slot per entry, on a band scale.
/// assert_eq!(XKind::default(), XKind::Category);
/// ```
///
/// [`XKind::Time`] carries day numbers (see [`crate::date`]), so ticks snap to
/// month and quarter boundaries rather than to a fixed count of days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum XKind {
    /// Names. There is no "between" — a band scale, one slot per entry.
    #[default]
    Category,
    /// Plain numbers on a continuous axis.
    Numeric,
    /// Day numbers (see [`crate::date`]) on a continuous axis.
    Time,
}

/// One tick: a value, where it landed, and what it says.
///
/// Ticks are computed once per layout and reused by the axis labels, the
/// gridlines, and the accessibility summary, so the three can never disagree.
///
/// ```
/// use silka_chart::model::Tick;
///
/// let tick = Tick { value: 1.5e6, position: 84.0, label: "1,5 jt".into() };
/// assert_eq!(tick.label, "1,5 jt");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    /// The data value.
    pub value: f64,
    /// The position along its axis, in the node's local coordinates.
    pub position: f32,
    /// The formatted label.
    pub label: String,
}

/// One resolved series: a name, plain values, and an optional color override.
///
/// ```
/// use silka_chart::model::Series;
///
/// // A missing value is `NaN`, not zero — a gap in the line and a zero mean
/// // very different things, and only one of them is honest.
/// let s = Series::new("Income", vec![1.0, f64::NAN, 3.0]);
/// assert_eq!(s.value(0), Some(1.0));
/// assert_eq!(s.value(1), None);
/// assert_eq!(s.value(99), None); // past the end reads the same as no data
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// The name shown in the legend and the tooltip.
    pub name: String,
    /// The values, one per x position. Shorter series are padded with `NaN`,
    /// which reads as "no data here" all the way through — a gap in the line
    /// rather than a zero, because those mean very different things.
    pub values: Vec<f64>,
    /// An explicit color, overriding the palette slot.
    pub color: Option<silka_paint::Color>,
}

impl Series {
    /// A named series.
    pub fn new(name: impl Into<String>, values: Vec<f64>) -> Self {
        Self {
            name: name.into(),
            values,
            color: None,
        }
    }

    /// The value at `index`, or `None` where there is no data.
    pub fn value(&self, index: usize) -> Option<f64> {
        self.values.get(index).copied().filter(|v| v.is_finite())
    }
}

/// The whole dataset, already resolved out of the caller's row type.
///
/// The builder's closures have already run by this point: whatever the
/// application's row type was, the chart itself only ever sees numbers.
///
/// ```
/// use silka_chart::model::{ChartData, Series};
///
/// let data = ChartData {
///     x: vec![0.0, 1.0, 2.0],
///     labels: vec!["Jan".into(), "Feb".into(), "Mar".into()],
///     series: vec![
///         Series::new("Income", vec![3.0, 5.0, 4.0]),
///         Series::new("Outgoing", vec![1.0, 2.0, 2.0]),
///     ],
/// };
/// assert_eq!(data.len(), 3);
///
/// // Grouped bars need the largest single value…
/// assert_eq!(data.value_domain(false).1, 5.0);
/// // …stacked bars need the largest total, which is a different number.
/// assert_eq!(data.value_domain(true).1, 7.0);
/// assert_eq!(data.stacked_totals(1), (0.0, 7.0));
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartData {
    /// The x position of each point (an index for categories, a number, or a
    /// day number).
    pub x: Vec<f64>,
    /// The label of each point — the category axis and the tooltip title.
    pub labels: Vec<String>,
    /// The series.
    pub series: Vec<Series>,
}

impl ChartData {
    /// How many x positions there are.
    pub fn len(&self) -> usize {
        self.x.len()
    }

    /// True when there is nothing to draw — which is a **state**, not an
    /// error, and gets its own visual (`KOMPONEN.md` Definition of Done).
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
            || self
                .series
                .iter()
                .all(|s| s.values.iter().all(|v| !v.is_finite()))
    }

    /// The x domain, for a continuous x axis.
    pub fn x_domain(&self) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for v in self.x.iter().filter(|v| v.is_finite()) {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        if lo > hi {
            (0.0, 1.0)
        } else {
            (lo, hi)
        }
    }

    /// The value domain.
    ///
    /// Stacking changes the answer and must not be an afterthought: a stacked
    /// chart's axis has to reach the **sum** at each x, not the largest single
    /// segment, or the top of the stack is drawn outside the plot. Positive and
    /// negative parts are stacked separately — the way every stacked chart
    /// worth reading does it — so a series that goes negative grows downward
    /// from zero instead of eating into the positive pile.
    pub fn value_domain(&self, stacked: bool) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        if stacked {
            for i in 0..self.len() {
                let (neg, pos) = self.stacked_totals(i);
                lo = lo.min(neg);
                hi = hi.max(pos);
            }
        } else {
            for s in &self.series {
                for v in s.values.iter().filter(|v| v.is_finite()) {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
            }
        }
        if lo > hi {
            (0.0, 1.0)
        } else {
            (lo, hi)
        }
    }

    /// The negative and positive stack totals at `index`.
    pub fn stacked_totals(&self, index: usize) -> (f64, f64) {
        let mut neg = 0.0;
        let mut pos = 0.0;
        for s in &self.series {
            match s.value(index) {
                Some(v) if v < 0.0 => neg += v,
                Some(v) => pos += v,
                None => {}
            }
        }
        (neg, pos)
    }

    /// The stacked base of `series` at `index` — where its segment starts.
    ///
    /// Only series on the **same side of zero** stack on top of one another.
    pub fn stack_base(&self, series: usize, index: usize) -> f64 {
        let Some(value) = self.series.get(series).and_then(|s| s.value(index)) else {
            return 0.0;
        };
        let negatif = value < 0.0;
        self.series[..series]
            .iter()
            .filter_map(|s| s.value(index))
            .filter(|v| (*v < 0.0) == negatif)
            .sum()
    }

    /// The label of point `index`, falling back to the x value.
    pub fn label(&self, index: usize, locale: &Locale, format: &NumberFormat) -> String {
        if let Some(l) = self.labels.get(index) {
            if !l.is_empty() {
                return l.clone();
            }
        }
        match self.x.get(index) {
            Some(x) => format.format(*x, locale),
            None => String::new(),
        }
    }
}

/// Everything about a chart that is not its data: which marks, which axes,
/// which formats.
///
/// ```
/// use silka_chart::model::{ChartKind, ChartSpec};
///
/// // Sparklines are word-sized: no axes, no grid, no legend — and that is a
/// // property of the kind, not something every caller has to switch off.
/// let spark = ChartSpec::new(ChartKind::Sparkline);
/// assert!(!spark.grid && !spark.value_axis && !spark.category_axis);
///
/// // "Zero-based" defaults honestly rather than uniformly: a truncated bar
/// // chart lies about its proportions, a truncated line chart does not.
/// assert!(ChartSpec::new(ChartKind::Bar).is_zero_based());
/// assert!(!ChartSpec::new(ChartKind::Line).is_zero_based());
///
/// // An explicit choice always wins over the default.
/// let mut line = ChartSpec::new(ChartKind::Line);
/// line.zero_based = Some(true);
/// assert!(line.is_zero_based());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ChartSpec {
    /// Which marks are drawn.
    pub kind: ChartKind,
    /// How several bar series share a category.
    pub bar_layout: BarLayout,
    /// Which way the value axis runs.
    pub orientation: Orientation,
    /// What the x values mean.
    pub x_kind: XKind,
    /// How value-axis labels and tooltip values are written.
    pub value_format: NumberFormat,
    /// How category-axis labels are written.
    pub category_format: NumberFormat,
    /// Who is reading.
    pub locale: Locale,
    /// Draw gridlines behind the marks.
    pub grid: bool,
    /// Draw the category axis with its labels.
    pub category_axis: bool,
    /// Draw the value axis with its labels.
    pub value_axis: bool,
    /// Show the legend.
    pub legend: bool,
    /// Draw a marker at each data point.
    pub markers: bool,
    /// Animate value changes on a spring.
    pub animated: bool,
    /// Force the value axis to include zero. `None` = the honest default:
    /// bars yes, lines no.
    pub zero_based: Option<bool>,
    /// The chart's title, which is also its accessibility name.
    pub title: Option<String>,
    /// What is shown when there is no data.
    pub empty_message: String,
}

impl ChartSpec {
    /// The default spec for a kind.
    pub fn new(kind: ChartKind) -> Self {
        let sparkline = kind == ChartKind::Sparkline;
        Self {
            kind,
            bar_layout: BarLayout::default(),
            orientation: Orientation::default(),
            x_kind: if kind.is_bar() {
                XKind::Category
            } else {
                XKind::Numeric
            },
            value_format: NumberFormat::Auto,
            category_format: NumberFormat::Auto,
            locale: Locale::EN_US,
            grid: !sparkline,
            category_axis: !sparkline,
            value_axis: !sparkline,
            legend: false,
            markers: false,
            animated: true,
            zero_based: None,
            title: None,
            empty_message: "No data".to_string(),
        }
    }

    /// Whether the value axis includes zero, after the default is applied.
    pub fn is_zero_based(&self) -> bool {
        self.zero_based.unwrap_or_else(|| self.kind.is_bar())
    }

    /// Whether the marks stack.
    pub fn is_stacked(&self) -> bool {
        self.kind.is_bar() && self.bar_layout == BarLayout::Stacked
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The category axis: a band for names, a linear scale for numbers and time.
///
/// Two shapes behind one interface, because "where does point 3 go?" has to be
/// answerable without the caller knowing which kind of axis it is asking.
///
/// ```
/// use silka_chart::model::CategoryScale;
/// use silka_chart::scale::{BandScale, LinearScale};
///
/// let names = CategoryScale::Band(BandScale::new(4, 0.0, 400.0));
/// let numbers = CategoryScale::Linear(LinearScale::new(0.0, 3.0, 0.0, 400.0));
///
/// // A band centres its slot; a linear scale maps the x value itself.
/// assert!(names.position(0, 0.0) > 0.0);
/// assert_eq!(numbers.position(0, 0.0), 0.0);
///
/// // Both can say how much room one category's marks may take.
/// assert!(names.band_width(4) > 0.0);
/// assert!(numbers.band_width(4) > 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CategoryScale {
    /// One slot per entry, with gaps — categories have no "between".
    Band(BandScale),
    /// A continuous position — numeric and time axes.
    Linear(LinearScale),
}

impl CategoryScale {
    /// Where point `index` (with x value `x`) sits along the category axis.
    pub fn position(&self, index: usize, x: f64) -> f32 {
        match self {
            CategoryScale::Band(b) => b.center(index),
            CategoryScale::Linear(l) => l.map(x),
        }
    }

    /// The width available to one category's marks.
    pub fn band_width(&self, count: usize) -> f32 {
        match self {
            CategoryScale::Band(b) => b.band_width(),
            CategoryScale::Linear(l) => {
                let (a, b) = l.range();
                let span = (b - a).abs();
                if count > 1 {
                    span / (count - 1) as f32 * 0.8
                } else {
                    span * 0.5
                }
            }
        }
    }
}

/// The full plot geometry: where everything goes, in the node's local
/// coordinates.
///
/// A pure function of `(plot, spec, data)` — which is what makes positions
/// assertable without a window or a GPU.
///
/// ```
/// use silka_paint::{Point, Rect};
/// use silka_chart::model::{ChartData, ChartKind, ChartSpec, PlotGeometry, Series};
///
/// let data = ChartData {
///     x: vec![0.0, 1.0, 2.0],
///     labels: vec!["Jan".into(), "Feb".into(), "Mar".into()],
///     series: vec![Series::new("Income", vec![1.0, 5.0, 3.0])],
/// };
/// let spec = ChartSpec::new(ChartKind::Bar);
/// let geometry = PlotGeometry::build(Rect::new(0.0, 0.0, 300.0, 200.0), &spec, &data);
///
/// // A bar chart's axis reaches zero, so the baseline sits at the bottom.
/// assert_eq!(geometry.baseline, 200.0);
/// assert!(!geometry.value_ticks.is_empty());
///
/// // Values map upward: the larger value is higher on the screen.
/// let low = geometry.point(0, 0.0, 1.0);
/// let high = geometry.point(1, 1.0, 5.0);
/// assert!(high.y < low.y);
///
/// // Hover hit-testing is the inverse of the same geometry.
/// assert_eq!(geometry.index_at(Point::new(high.x, 100.0), &data), Some(1));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PlotGeometry {
    /// The data area — the rect the marks are clipped to.
    pub plot: Rect,
    /// Which way the value axis runs.
    pub orientation: Orientation,
    /// Data magnitude → screen coordinate on the value axis.
    pub value: LinearScale,
    /// Point index / x value → screen coordinate on the category axis.
    pub category: CategoryScale,
    /// The value-axis ticks (and therefore the gridlines).
    pub value_ticks: Vec<Tick>,
    /// The category-axis ticks.
    pub category_ticks: Vec<Tick>,
    /// The distance between two value ticks, which decides label decimals.
    pub value_step: f64,
    /// Where zero sits on the value axis, clamped into the plot.
    pub baseline: f32,
}

impl PlotGeometry {
    /// Build the geometry for a plot rect.
    ///
    /// Everything here is a pure function of `(plot, spec, data)` — which is
    /// what makes the tests below able to assert on positions without a window
    /// or a GPU.
    pub fn build(plot: Rect, spec: &ChartSpec, data: &ChartData) -> Self {
        let horizontal = spec.orientation == Orientation::Horizontal;
        // The value axis runs bottom-to-top when vertical (screen y is
        // inverted) and left-to-right when horizontal.
        let (value_start, value_end, value_extent) = if horizontal {
            (plot.min_x(), plot.max_x(), plot.size.width)
        } else {
            (plot.max_y(), plot.min_y(), plot.size.height)
        };
        let (cat_start, cat_end, cat_extent) = if horizontal {
            (plot.min_y(), plot.max_y(), plot.size.height)
        } else {
            (plot.min_x(), plot.max_x(), plot.size.width)
        };

        // How much room one tick needs depends on which way its labels lie,
        // not on which axis it is. Whichever axis runs **horizontally** has its
        // labels side by side and needs their width; the vertical one has them
        // stacked and needs only their height. Using one figure for both is the
        // easy mistake, and it shows up as a tall plot with two gridlines.
        let (value_spacing, category_spacing) = if horizontal {
            (MIN_TICK_SPACING, MIN_STACKED_TICK_SPACING)
        } else {
            (MIN_STACKED_TICK_SPACING, MIN_TICK_SPACING * 1.4)
        };

        // -- value axis ----------------------------------------------------
        let (raw_lo, raw_hi) = data.value_domain(spec.is_stacked());
        let target = ticks::tick_count_for(value_extent, value_spacing);
        let (lo, hi) = if spec.is_zero_based() {
            ticks::zero_based_domain(raw_lo, raw_hi, target)
        } else {
            ticks::nice_domain(raw_lo, raw_hi, target)
        };
        let value = LinearScale::new(lo, hi, value_start, value_end);
        let nilai_tick = ticks::nice_ticks(lo, hi, target);
        let value_step = nilai_tick
            .windows(2)
            .map(|w| w[1] - w[0])
            .next()
            .unwrap_or(0.0);
        // `format_axis`, not `format_tick`: every label on one axis has to speak
        // the same unit, and only the axis knows how large it gets.
        let rentang = lo.abs().max(hi.abs());
        let value_ticks = nilai_tick
            .iter()
            .map(|v| Tick {
                value: *v,
                position: value.map(*v),
                label: spec
                    .value_format
                    .format_axis(*v, value_step, rentang, &spec.locale),
            })
            .collect();

        // -- category axis -------------------------------------------------
        let category = match spec.x_kind {
            XKind::Category => CategoryScale::Band(
                BandScale::new(data.len(), cat_start, cat_end)
                    // Grouped bars split the band between them; without a
                    // little extra room the groups touch across categories and
                    // the reader cannot see where one category ends.
                    .padding_inner(if spec.is_stacked() { 0.2 } else { 0.25 }),
            ),
            XKind::Numeric | XKind::Time => {
                let (x_lo, x_hi) = data.x_domain();
                CategoryScale::Linear(LinearScale::new(x_lo, x_hi, cat_start, cat_end))
            }
        };
        let category_ticks =
            build_category_ticks(spec, data, &category, cat_extent, category_spacing);

        Self {
            plot,
            orientation: spec.orientation,
            value,
            category,
            value_ticks,
            category_ticks,
            value_step,
            baseline: value.map_clamped(0.0f64.clamp(lo, hi)),
        }
    }

    /// The screen point of one data value.
    ///
    /// Both orientations go through here, which is why nothing downstream has
    /// to know which one is in effect.
    pub fn point(&self, index: usize, x: f64, value: f64) -> Point {
        let c = self.category.position(index, x);
        let v = self.value.map(value);
        match self.orientation {
            Orientation::Vertical => Point::new(c, v),
            Orientation::Horizontal => Point::new(v, c),
        }
    }

    /// The rect of one bar, from `base` to `value` across `width` of its band.
    ///
    /// Handles negative values (the bar hangs the other way) and the
    /// zero-height case (a value of exactly zero still gets a hairline, so a
    /// category with no value is visibly *present* rather than missing).
    pub fn bar_rect(&self, center: f32, width: f32, base: f64, value: f64) -> Rect {
        let a = self.value.map(base);
        let b = self.value.map(value);
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let width = width.max(1.0);
        match self.orientation {
            Orientation::Vertical => Rect::new(center - width * 0.5, lo, width, (hi - lo).max(1.0)),
            Orientation::Horizontal => {
                Rect::new(lo, center - width * 0.5, (hi - lo).max(1.0), width)
            }
        }
    }

    /// Which data point is nearest to a position inside the plot.
    ///
    /// The whole plot answers, not just the marks: a reader hovering the empty
    /// space above a short bar means that bar, and a tooltip that only appears
    /// when the pointer is exactly on a 2pt line is a tooltip nobody will ever
    /// see.
    pub fn index_at(&self, position: Point, data: &ChartData) -> Option<usize> {
        if data.is_empty() {
            return None;
        }
        let along = match self.orientation {
            Orientation::Vertical => position.x,
            Orientation::Horizontal => position.y,
        };
        match &self.category {
            CategoryScale::Band(b) => b.index_at(along),
            CategoryScale::Linear(_) => {
                let mut terbaik = 0usize;
                let mut jarak = f32::INFINITY;
                for (i, x) in data.x.iter().enumerate() {
                    let d = (self.category.position(i, *x) - along).abs();
                    if d < jarak {
                        jarak = d;
                        terbaik = i;
                    }
                }
                Some(terbaik)
            }
        }
    }

    /// A gridline across the plot at a value-axis position.
    pub fn value_gridline(&self, position: f32, thickness: f32) -> Rect {
        let t = thickness.max(0.5);
        match self.orientation {
            Orientation::Vertical => Rect::new(
                self.plot.min_x(),
                position - t * 0.5,
                self.plot.size.width,
                t,
            ),
            Orientation::Horizontal => Rect::new(
                position - t * 0.5,
                self.plot.min_y(),
                t,
                self.plot.size.height,
            ),
        }
    }

    /// A gridline across the plot at a category-axis position.
    pub fn category_gridline(&self, position: f32, thickness: f32) -> Rect {
        let t = thickness.max(0.5);
        match self.orientation {
            Orientation::Vertical => Rect::new(
                position - t * 0.5,
                self.plot.min_y(),
                t,
                self.plot.size.height,
            ),
            Orientation::Horizontal => Rect::new(
                self.plot.min_x(),
                position - t * 0.5,
                self.plot.size.width,
                t,
            ),
        }
    }
}

/// Ticks along the category axis — one per entry for names, generated for
/// numeric and time axes.
fn build_category_ticks(
    spec: &ChartSpec,
    data: &ChartData,
    scale: &CategoryScale,
    extent: f32,
    min_spacing: f32,
) -> Vec<Tick> {
    let target = ticks::tick_count_for(extent, min_spacing);
    match spec.x_kind {
        XKind::Category => {
            // Every category deserves a label, but labels that overlap are
            // worse than no labels: thin them out by a whole stride so the ones
            // that survive stay evenly spaced.
            let stride = if data.len() > target {
                (data.len() as f32 / target as f32).ceil() as usize
            } else {
                1
            };
            (0..data.len())
                .step_by(stride.max(1))
                .map(|i| Tick {
                    value: i as f64,
                    position: scale.position(i, data.x.get(i).copied().unwrap_or(i as f64)),
                    label: data.label(i, &spec.locale, &spec.category_format),
                })
                .collect()
        }
        XKind::Time => {
            let (lo, hi) = data.x_domain();
            let (unit, nilai) = ticks::time_ticks(lo, hi, target);
            nilai
                .iter()
                .map(|v| Tick {
                    value: *v,
                    position: scale.position(0, *v),
                    label: label_waktu(spec, *v, unit),
                })
                .collect()
        }
        XKind::Numeric => {
            let (lo, hi) = data.x_domain();
            let nilai = ticks::nice_ticks(lo, hi, target);
            let langkah = nilai.windows(2).map(|w| w[1] - w[0]).next().unwrap_or(0.0);
            nilai
                .iter()
                .map(|v| Tick {
                    value: *v,
                    position: scale.position(0, *v),
                    label: spec.category_format.format_tick(*v, langkah, &spec.locale),
                })
                .collect()
        }
    }
}

/// A time tick's label: the caller's explicit format wins, otherwise the unit
/// the tick generator chose decides.
fn label_waktu(spec: &ChartSpec, value: f64, unit: TimeUnit) -> String {
    match &spec.category_format {
        NumberFormat::Date(u) => spec.locale.date(value, *u),
        NumberFormat::Auto | NumberFormat::Category => spec.locale.date(value, unit),
        other => other.format(value, &spec.locale),
    }
}

/// The rect a chart would like, given loose constraints.
///
/// A chart has no natural size — it fills what it is given. What it does have
/// is a **minimum** below which it stops being readable, and a default for the
/// unbounded case, which is what this answers.
pub fn preferred_size(constraints_max: Size, kind: ChartKind) -> Size {
    let (default_w, default_h) = match kind {
        ChartKind::Sparkline => (96.0, 24.0),
        _ => (480.0, 280.0),
    };
    Size::new(
        if constraints_max.width.is_finite() {
            constraints_max.width
        } else {
            default_w
        },
        if constraints_max.height.is_finite() {
            constraints_max.height
        } else {
            default_h
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::Date;

    fn deret(nama: &str, v: &[f64]) -> Series {
        Series::new(nama, v.to_vec())
    }

    fn data_kategori() -> ChartData {
        ChartData {
            x: vec![0.0, 1.0, 2.0, 3.0],
            labels: vec!["Q1".into(), "Q2".into(), "Q3".into(), "Q4".into()],
            series: vec![
                deret("Pendapatan", &[10.0, 20.0, 15.0, 30.0]),
                deret("Biaya", &[5.0, 8.0, 12.0, 9.0]),
            ],
        }
    }

    fn spec(kind: ChartKind) -> ChartSpec {
        ChartSpec::new(kind)
    }

    fn plot() -> Rect {
        Rect::new(40.0, 10.0, 400.0, 200.0)
    }

    #[test]
    fn domain_bertumpuk_mencapai_jumlah_bukan_deret_terbesar() {
        // If the domain only reached the tallest single series, the top of
        // every stack would be drawn outside the plot.
        let d = data_kategori();
        let (_, tanpa) = d.value_domain(false);
        let (_, dengan) = d.value_domain(true);
        assert_eq!(tanpa, 30.0);
        assert_eq!(dengan, 39.0, "20+8 dan 30+9 → 39");
    }

    #[test]
    fn nilai_negatif_bertumpuk_ke_bawah_bukan_memakan_yang_positif() {
        let d = ChartData {
            x: vec![0.0],
            labels: vec!["A".into()],
            series: vec![deret("naik", &[10.0]), deret("turun", &[-4.0])],
        };
        assert_eq!(d.stacked_totals(0), (-4.0, 10.0));
        let (lo, hi) = d.value_domain(true);
        assert_eq!((lo, hi), (-4.0, 10.0));
        // The second series starts at zero, not at the top of the first one.
        assert_eq!(d.stack_base(1, 0), 0.0);
        assert_eq!(d.stack_base(0, 0), 0.0);
    }

    #[test]
    fn dasar_tumpukan_menjumlah_sisi_yang_sama_saja() {
        let d = ChartData {
            x: vec![0.0],
            labels: vec!["A".into()],
            series: vec![deret("a", &[10.0]), deret("b", &[-5.0]), deret("c", &[7.0])],
        };
        assert_eq!(
            d.stack_base(2, 0),
            10.0,
            "c menumpuk di atas a, bukan di atas a+b"
        );
        assert_eq!(d.stack_base(1, 0), 0.0);
    }

    #[test]
    fn nan_adalah_lubang_bukan_nol() {
        // The distinction that keeps a chart honest: a missing measurement is
        // a gap in the line, a measured zero is a point on the baseline.
        let s = deret("a", &[1.0, f64::NAN, 3.0]);
        assert_eq!(s.value(1), None);
        assert_eq!(s.value(2), Some(3.0));
        let d = ChartData {
            x: vec![0.0, 1.0, 2.0],
            labels: vec![],
            series: vec![s],
        };
        assert_eq!(d.value_domain(false), (1.0, 3.0));
        assert!(!d.is_empty());
    }

    #[test]
    fn data_kosong_terdeteksi_sebagai_keadaan_bukan_galat() {
        assert!(ChartData::default().is_empty());
        let semua_nan = ChartData {
            x: vec![0.0, 1.0],
            labels: vec![],
            series: vec![deret("a", &[f64::NAN, f64::NAN])],
        };
        assert!(semua_nan.is_empty());
    }

    #[test]
    fn sumbu_batang_menyertakan_nol_sumbu_garis_tidak() {
        // The rule that separates the two: a bar's *length* is its value, so a
        // truncated axis lies about magnitude. A line's *position* is its
        // value, and forcing zero onto 980–1010 flattens it to nothing.
        let d = ChartData {
            x: vec![0.0, 1.0],
            labels: vec!["a".into(), "b".into()],
            series: vec![deret("s", &[980.0, 1_010.0])],
        };
        let batang = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &d);
        assert_eq!(batang.value.domain().0, 0.0);

        let mut s = spec(ChartKind::Line);
        s.x_kind = XKind::Numeric;
        let garis = PlotGeometry::build(plot(), &s, &d);
        assert!(garis.value.domain().0 > 900.0, "{:?}", garis.value.domain());

        // …and the caller can still overrule either way.
        let mut paksa = spec(ChartKind::Line);
        paksa.x_kind = XKind::Numeric;
        paksa.zero_based = Some(true);
        assert_eq!(
            PlotGeometry::build(plot(), &paksa, &d).value.domain().0,
            0.0
        );
    }

    #[test]
    fn nilai_terbesar_berada_di_atas_plot() {
        let d = data_kategori();
        let g = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &d);
        let atas = g.value.map(g.value.domain().1);
        let bawah = g.value.map(g.value.domain().0);
        assert!(
            atas < bawah,
            "y layar terbalik: {atas} harus di atas {bawah}"
        );
        assert!((atas - plot().min_y()).abs() < 0.001);
        assert!((bawah - plot().max_y()).abs() < 0.001);
    }

    #[test]
    fn setiap_mark_berada_di_dalam_plot() {
        // The claim a chart makes to its reader: what is inside the box is the
        // data. Anything escaping the plot rect would be drawn over the axis
        // labels.
        let d = data_kategori();
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Area] {
            let mut s = spec(kind);
            if !kind.is_bar() {
                s.x_kind = XKind::Numeric;
            }
            let g = PlotGeometry::build(plot(), &s, &d);
            for (si, deret) in d.series.iter().enumerate() {
                for i in 0..d.len() {
                    let v = deret.value(i).unwrap();
                    let p = g.point(i, d.x[i], v);
                    assert!(
                        p.x >= plot().min_x() - 0.01 && p.x <= plot().max_x() + 0.01,
                        "{kind:?} deret {si} titik {i}: {p:?}"
                    );
                    assert!(
                        p.y >= plot().min_y() - 0.01 && p.y <= plot().max_y() + 0.01,
                        "{kind:?} deret {si} titik {i}: {p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn orientasi_horizontal_menukar_kedua_sumbu() {
        let d = data_kategori();
        let mut s = spec(ChartKind::Bar);
        s.orientation = Orientation::Horizontal;
        let g = PlotGeometry::build(plot(), &s, &d);

        // The value now grows to the right and the categories run downward.
        let kecil = g.point(0, 0.0, 10.0);
        let besar = g.point(0, 0.0, 30.0);
        assert!(besar.x > kecil.x, "nilai besar harus lebih ke kanan");
        assert!((besar.y - kecil.y).abs() < 0.001, "kategori sama, y sama");

        let kategori_0 = g.point(0, 0.0, 10.0);
        let kategori_3 = g.point(3, 3.0, 10.0);
        assert!(
            kategori_3.y > kategori_0.y,
            "kategori berikutnya lebih ke bawah"
        );
    }

    #[test]
    fn kotak_batang_menggantung_ke_arah_yang_benar() {
        let d = ChartData {
            x: vec![0.0, 1.0],
            labels: vec!["naik".into(), "turun".into()],
            series: vec![deret("s", &[20.0, -20.0])],
        };
        let g = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &d);
        let naik = g.bar_rect(100.0, 20.0, 0.0, 20.0);
        let turun = g.bar_rect(200.0, 20.0, 0.0, -20.0);
        assert!(
            naik.max_y() <= g.baseline + 0.01,
            "batang positif di atas nol"
        );
        assert!(
            turun.min_y() >= g.baseline - 0.01,
            "batang negatif di bawah nol"
        );
        // A value of exactly zero still leaves a hairline, so an empty category
        // reads as present rather than missing.
        let nol = g.bar_rect(300.0, 20.0, 0.0, 0.0);
        assert!(nol.size.height >= 1.0);
    }

    #[test]
    fn tick_nilai_berjarak_seragam_di_layar() {
        let g = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &data_kategori());
        assert!(g.value_ticks.len() >= 3);
        let jarak: Vec<f32> = g
            .value_ticks
            .windows(2)
            .map(|w| (w[1].position - w[0].position).abs())
            .collect();
        for j in &jarak {
            assert!((j - jarak[0]).abs() < 0.01, "{jarak:?}");
        }
        for t in &g.value_ticks {
            assert!(!t.label.is_empty());
        }
    }

    #[test]
    fn sumbu_tegak_muat_lebih_banyak_tick_daripada_sumbu_mendatar() {
        // A vertical axis stacks its labels, so it fits far more of them in the
        // same number of points than a horizontal one, whose labels sit side by
        // side. Charging both the same rent leaves a perfectly tall plot with
        // two gridlines and calls it an axis.
        let d = data_kategori();
        let sempit = Rect::new(40.0, 10.0, 400.0, 130.0);

        let tegak = PlotGeometry::build(sempit, &spec(ChartKind::Bar), &d);
        assert!(
            tegak.value_ticks.len() >= 3,
            "plot 130pt harus muat ≥3 gridline, bukan {}",
            tegak.value_ticks.len()
        );

        let mut s = spec(ChartKind::Bar);
        s.orientation = Orientation::Horizontal;
        let datar = PlotGeometry::build(Rect::new(40.0, 10.0, 130.0, 400.0), &s, &d);
        assert!(
            datar.value_ticks.len() <= tegak.value_ticks.len(),
            "sumbu nilai mendatar sepanjang 130pt tidak boleh sepadat yang tegak"
        );
    }

    #[test]
    fn kategori_mendatar_tidak_kehilangan_namanya() {
        // Four cost categories down a 160pt tall plot: every one keeps its
        // label. Reusing the side-by-side spacing here dropped half of them,
        // which is the whole reason `horizontal()` exists — long names.
        let d = data_kategori();
        let mut s = spec(ChartKind::Bar);
        s.orientation = Orientation::Horizontal;
        let g = PlotGeometry::build(Rect::new(80.0, 10.0, 300.0, 160.0), &s, &d);
        assert_eq!(g.category_ticks.len(), d.len(), "{:?}", g.category_ticks);
    }

    #[test]
    fn label_kategori_dijarangkan_saat_tak_muat() {
        // Twelve categories in 400pt would overlap; thinning by a whole stride
        // keeps the survivors evenly spaced, which unequal thinning would not.
        let d = ChartData {
            x: (0..40).map(|i| i as f64).collect(),
            labels: (0..40).map(|i| format!("Kategori {i}")).collect(),
            series: vec![Series::new("s", (0..40).map(|i| i as f64).collect())],
        };
        let g = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &d);
        assert!(g.category_ticks.len() < 40, "{}", g.category_ticks.len());
        assert!(!g.category_ticks.is_empty());
        let jarak: Vec<f32> = g
            .category_ticks
            .windows(2)
            .map(|w| w[1].position - w[0].position)
            .collect();
        for j in &jarak {
            assert!((j - jarak[0]).abs() < 0.01, "{jarak:?}");
        }
    }

    #[test]
    fn sumbu_waktu_diberi_label_tanggal_bukan_angka_hari() {
        let hari: Vec<f64> = (0..365)
            .step_by(7)
            .map(|d| (Date::new(2026, 1, 1).to_days() + d) as f64)
            .collect();
        let d = ChartData {
            x: hari.clone(),
            labels: vec![],
            series: vec![Series::new("s", hari.iter().map(|_| 1.0).collect())],
        };
        let mut s = spec(ChartKind::Line);
        s.x_kind = XKind::Time;
        let g = PlotGeometry::build(plot(), &s, &d);
        assert!(!g.category_ticks.is_empty());
        for t in &g.category_ticks {
            assert!(
                t.label.chars().any(|c| c.is_alphabetic()),
                "label sumbu waktu harus berupa tanggal, bukan {}",
                t.label
            );
        }
    }

    #[test]
    fn hover_menjawab_di_mana_pun_di_dalam_plot() {
        // A tooltip that only fires when the pointer is exactly on a 2pt line
        // is a tooltip nobody will ever see.
        let d = data_kategori();
        let g = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &d);
        let tengah_atas = Point::new(g.category.position(2, 2.0), plot().min_y() + 2.0);
        assert_eq!(g.index_at(tengah_atas, &d), Some(2));
        let tengah_bawah = Point::new(g.category.position(2, 2.0), plot().max_y() - 2.0);
        assert_eq!(g.index_at(tengah_bawah, &d), Some(2));
    }

    #[test]
    fn hover_pada_sumbu_menerus_memungut_titik_terdekat() {
        let d = data_kategori();
        let mut s = spec(ChartKind::Line);
        s.x_kind = XKind::Numeric;
        let g = PlotGeometry::build(plot(), &s, &d);
        for i in 0..d.len() {
            let p = g.point(i, d.x[i], 10.0);
            assert_eq!(g.index_at(p, &d), Some(i), "titik {i}");
        }
        // Between two points, the nearer one wins.
        let a = g.point(0, 0.0, 10.0);
        let b = g.point(1, 1.0, 10.0);
        let hampir_b = Point::new(a.x + (b.x - a.x) * 0.8, 100.0);
        assert_eq!(g.index_at(hampir_b, &d), Some(1));
    }

    #[test]
    fn hover_pada_data_kosong_tidak_panik() {
        let d = ChartData::default();
        let g = PlotGeometry::build(plot(), &spec(ChartKind::Bar), &d);
        assert_eq!(g.index_at(Point::new(100.0, 100.0), &d), None);
    }

    #[test]
    fn gridline_melintasi_plot_di_kedua_orientasi() {
        let d = data_kategori();
        for (orient, _) in [(Orientation::Vertical, 0), (Orientation::Horizontal, 1)] {
            let mut s = spec(ChartKind::Bar);
            s.orientation = orient;
            let g = PlotGeometry::build(plot(), &s, &d);
            let r = g.value_gridline(g.value_ticks[1].position, 1.0);
            let panjang = match orient {
                Orientation::Vertical => r.size.width,
                Orientation::Horizontal => r.size.height,
            };
            let sisi = match orient {
                Orientation::Vertical => plot().size.width,
                Orientation::Horizontal => plot().size.height,
            };
            assert!((panjang - sisi).abs() < 0.01, "{orient:?}: {r:?}");
        }
    }

    #[test]
    fn baseline_dijepit_ke_dalam_plot() {
        // A series that never reaches zero must not put its baseline outside
        // the plot, where it would be drawn over the axis labels.
        let d = ChartData {
            x: vec![0.0, 1.0],
            labels: vec!["a".into(), "b".into()],
            series: vec![deret("s", &[980.0, 1_010.0])],
        };
        let mut s = spec(ChartKind::Line);
        s.x_kind = XKind::Numeric;
        let g = PlotGeometry::build(plot(), &s, &d);
        assert!(g.baseline >= plot().min_y() - 0.01 && g.baseline <= plot().max_y() + 0.01);
    }

    #[test]
    fn ukuran_pilihan_mengisi_yang_diberikan_dan_punya_cadangan() {
        let terikat = preferred_size(Size::new(300.0, 150.0), ChartKind::Line);
        assert_eq!(terikat, Size::new(300.0, 150.0));
        let bebas = preferred_size(Size::new(f32::INFINITY, f32::INFINITY), ChartKind::Line);
        assert!(bebas.width > 0.0 && bebas.height > 0.0);
        let percikan = preferred_size(
            Size::new(f32::INFINITY, f32::INFINITY),
            ChartKind::Sparkline,
        );
        assert!(percikan.height < bebas.height, "sparkline seukuran kata");
    }

    #[test]
    fn spec_default_masuk_akal_per_jenis() {
        assert!(ChartSpec::new(ChartKind::Bar).is_zero_based());
        assert!(!ChartSpec::new(ChartKind::Line).is_zero_based());
        assert!(!ChartSpec::new(ChartKind::Sparkline).grid);
        assert!(!ChartSpec::new(ChartKind::Sparkline).value_axis);
        assert_eq!(ChartSpec::new(ChartKind::Bar).x_kind, XKind::Category);
    }
}
