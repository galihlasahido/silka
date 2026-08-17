//! **The chart render node** — where geometry, springs, text, and input meet.
//!
//! One node draws the whole chart. That is a deliberate choice and worth
//! justifying, because the alternative (a node per axis, per series, per label)
//! is what a widget toolkit's instincts suggest:
//!
//! - **Axis space is circular.** How wide the value axis must be depends on how
//!   long its labels are, which depends on the tick values, which depend on how
//!   tall the plot is, which depends on the category axis's labels. Resolving
//!   that as a parent/child layout means a node reaching into its sibling's
//!   measurements — precisely what box constraints forbid. Resolving it inside
//!   one node is two passes and a comment (see `ChartBox::rebuild`).
//! - **A chart is one thing to a reader and one node to a screen reader.**
//!   Eight hundred child nodes, one per data point, would be eight hundred
//!   items to tab through.
//!
//! What that costs is that text has to be rasterised **in layout** rather than
//! in paint — `paint` takes `&self` and glyph rasterisation needs the atlas
//! mutably. That is the same seam [`silka_widgets::text()`] sits on, for the same
//! reason, and it has a pleasant consequence: hovering re-paints without
//! re-shaping a single label.
//!
//! ## What is drawn, in order
//!
//! Background → gridlines → zero rule → **marks** → crosshair → axis rules →
//! labels → legend. Marks below the crosshair so the pointer never hides the
//! data; labels above everything so a tall bar never covers its own axis.
//!
//! ```
//! use silka_chart::{line_chart_in, ChartBox};
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::reconcile;
//! use silka_paint::Size;
//! use silka_theme::{Appearance, Theme};
//! use silka_widgets::Fonts;
//!
//! let fonts = Fonts::bundled_only();
//! let theme = Theme::cupertino(Appearance::Dark);
//!
//! let mut tree = RenderTree::new();
//! reconcile(
//!     &mut tree,
//!     line_chart_in(&fonts, &theme, [10.0f64, 30.0, 20.0])
//!         .numeric()
//!         .y_named("Value", |v: &f64| *v)
//!         .title("Throughput"),
//! );
//! tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
//!
//! // The entire chart is one node: not one per axis, per series, per label.
//! let id = tree.children(tree.root())[0];
//! let chart = tree.node_ref::<ChartBox>(id).expect("one node per chart");
//! assert_eq!(tree.children(id).len(), 0);
//!
//! // Which is also what a screen reader gets — a sentence describing the
//! // shape of the data, rather than eight hundred points to tab through.
//! let summary = chart.summary();
//! assert_eq!(summary, "line chart, 1 series, 3 points, 10 to 30");
//!
//! // Nothing is hovered until a pointer says so.
//! assert_eq!(chart.hovered(), None);
//! ```

use std::rc::Rc;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick as AnimTick};
use silka_core::input::{Event, EventCtx, HitBehavior, PointerPhase};
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{
    Color, Corners, GlyphRun, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke,
};
use silka_text::{TextConstraints, TextStyle};
use silka_widgets::Fonts;

use crate::model::{
    preferred_size, CategoryScale, ChartData, ChartKind, ChartSpec, Orientation, PlotGeometry,
};
use crate::stroke::{self, COLUMN_STEP};
use crate::style::ChartStyle;
use crate::tooltip::{ChartHover, HoverEntry};

/// The callback an application receives when the pointer moves over the plot.
pub type HoverCallback = Rc<dyn Fn(Option<ChartHover>)>;

/// A label already shaped and placed, waiting for paint.
#[derive(Debug, Clone)]
struct Placed {
    run: GlyphRun,
}

/// A legend swatch: its box and its color.
#[derive(Debug, Clone, Copy)]
struct Swatch {
    rect: Rect,
    color: Color,
}

/// The render node behind every chart in this crate.
///
/// **One node for the whole chart**, deliberately. Axis space is circular — the
/// value axis's width depends on labels that depend on ticks that depend on the
/// plot height that depends on the category axis — and box constraints rightly
/// forbid a node from reading its sibling's measurements. Resolving the
/// circularity inside a single node is two passes and a comment; spreading it
/// across sibling nodes is not possible at all.
///
/// It carries the same contracts every widget does: springs for data
/// transitions ([`ChartBox::advance`]), and an accessibility node whose name is
/// a real description of the content rather than a bare "image"
/// ([`ChartBox::summary`]).
pub struct ChartBox {
    pub(crate) spec: ChartSpec,
    pub(crate) data: ChartData,
    pub(crate) style: ChartStyle,
    pub(crate) fonts: Fonts,
    pub(crate) on_hover: Option<HoverCallback>,

    // -- derived (a pure product of the fields above plus the node size) ----
    size: Size,
    geometry: Option<PlotGeometry>,
    labels: Vec<Placed>,
    swatches: Vec<Swatch>,
    /// The node size the derived state was built for; `NaN` forces a rebuild.
    built_for: Size,
    /// The scale factor the glyphs were rasterised at — atlas bitmaps are tied
    /// to it (§3.3).
    built_scale: f32,
    /// Set by `update` when the data or the spec changed under us.
    stale: bool,

    // -- animation ---------------------------------------------------------
    /// One spring per (series, point), in **units of [`ChartBox::value_scale`]**
    /// rather than in the data's own units. The chart animates values, not the
    /// axis: re-shaping every tick label sixty times a second to make the axis
    /// glide would cost far more than it buys, and a moving axis makes the data
    /// look like it is moving when it is not.
    anim: Vec<Vec<SpringValue<f32>>>,
    /// What one spring unit is worth in data units — see
    /// [`ChartBox::sync_springs`].
    value_scale: f64,

    // -- input -------------------------------------------------------------
    hover: Option<usize>,
}

impl ChartBox {
    /// A fresh node from a spec plus data.
    pub(crate) fn new(
        spec: ChartSpec,
        data: ChartData,
        style: ChartStyle,
        fonts: Fonts,
        on_hover: Option<HoverCallback>,
    ) -> Self {
        let mut node = Self {
            spec,
            data,
            style,
            fonts,
            on_hover,
            size: Size::ZERO,
            geometry: None,
            labels: Vec::new(),
            swatches: Vec::new(),
            built_for: Size::new(f32::NAN, f32::NAN),
            built_scale: f32::NAN,
            stale: true,
            anim: Vec::new(),
            value_scale: 1.0,
            hover: None,
        };
        node.sync_springs(true);
        node
    }

    /// The plot geometry from the last layout — the door tests use to assert on
    /// positions.
    pub fn geometry(&self) -> Option<&PlotGeometry> {
        self.geometry.as_ref()
    }

    /// The data currently displayed.
    pub fn data(&self) -> &ChartData {
        &self.data
    }

    /// The point currently under the pointer.
    pub fn hovered(&self) -> Option<usize> {
        self.hover
    }

    /// Mark the derived state as stale — called by the view layer after it has
    /// replaced the data or the spec.
    pub(crate) fn invalidate(&mut self) {
        self.stale = true;
    }

    // -- animation ---------------------------------------------------------

    /// Resize the spring grid to match the data and retarget every value.
    ///
    /// `initial` distinguishes the two cases that look identical afterwards but
    /// must not behave the same: on **first build** the marks grow out of the
    /// baseline (which is what makes a chart feel like it arrived rather than
    /// blinked), while on a **data change** each value springs from where it
    /// already was to where it now belongs, carrying its velocity (§3.5).
    ///
    /// ## Why the springs hold normalised values
    ///
    /// A [`SpringValue`] decides it has arrived by an **absolute** tolerance —
    /// [`Tolerance::POINTS`] is 1/512, chosen for logical points, where it is a
    /// fraction of a pixel. Feed it rupiah and that promise inverts: a spring
    /// heading for 1.5 billion has to land within 1/512 of it, which `f32`
    /// cannot even represent (its resolution up there is about 64). The spring
    /// would never settle, the frame scheduler would never idle, and the GPU
    /// would spin forever on a chart that had visibly stopped moving — the
    /// exact failure §3.5 exists to prevent, arriving through the one door
    /// nobody watches.
    ///
    /// So a spring here holds `value / value_scale`, where the scale is the
    /// largest magnitude in the data. The tolerance then means "1/512 of the
    /// tallest thing on the chart", which across a plot a few hundred points
    /// tall is comfortably under one pixel — the unit it was designed for. When
    /// the scale changes, existing springs are converted rather than reset, so a
    /// dataset swap mid-flight still carries its velocity.
    pub(crate) fn sync_springs(&mut self, initial: bool) {
        let animated = self.spec.animated;
        let spring = Spring::smooth();
        let skala = value_scale(&self.data);
        let konversi = (self.value_scale / skala) as f32;
        self.value_scale = skala;

        self.anim.resize_with(self.data.series.len(), Vec::new);
        self.anim.truncate(self.data.series.len());

        for (si, series) in self.data.series.iter().enumerate() {
            let baris = &mut self.anim[si];
            let n = series.values.len();
            if baris.len() > n {
                baris.truncate(n);
            }
            if konversi != 1.0 && konversi.is_finite() {
                for v in baris.iter_mut() {
                    let posisi = v.position() * konversi;
                    let laju = v.velocity() * konversi;
                    v.jump_to(posisi);
                    v.set_velocity(laju);
                }
            }
            while baris.len() < n {
                // A point that did not exist a frame ago has no history to
                // carry: it starts at zero and springs to its value, which
                // reads as growth rather than as a jump.
                let mut v = SpringValue::new(0.0).with_spring(spring);
                v.set_role(MotionRole::Essential);
                baris.push(v);
            }
            for (i, spring_value) in baris.iter_mut().enumerate() {
                let mentah = series.values[i];
                // A gap animates to zero rather than to `NaN`: a spring fed a
                // NaN target never settles either, and the mark is not drawn at
                // all (see `drawn_value`), so where it "goes" is invisible.
                let target = if mentah.is_finite() {
                    (mentah / skala) as f32
                } else {
                    0.0
                };
                if animated && !initial {
                    spring_value.set_target(target);
                } else if animated && initial {
                    spring_value.jump_to(0.0);
                    spring_value.set_target(target);
                } else {
                    spring_value.jump_to(target);
                }
            }
        }
    }

    /// The value to *draw* for one point — the spring's position, not the
    /// datum.
    ///
    /// The tooltip deliberately reads the datum instead: a number counting up
    /// while its spring settles is unreadable, and a reader who stops to look
    /// wants the value, not the animation.
    fn drawn_value(&self, series: usize, index: usize) -> Option<f64> {
        self.data.series.get(series)?.value(index)?;
        let v = self.anim.get(series)?.get(index)?.position();
        v.is_finite().then_some(v as f64 * self.value_scale)
    }

    /// Advance every spring one frame. Returns whether anything moved.
    pub fn advance(&mut self, tick: &AnimTick) -> bool {
        let mut moved = false;
        for baris in &mut self.anim {
            for v in baris.iter_mut() {
                let sebelum = v.position();
                tick.advance(v);
                if (v.position() - sebelum).abs() > f32::EPSILON {
                    moved = true;
                }
            }
        }
        moved
    }

    /// True while any value is still travelling.
    pub fn is_animating(&self) -> bool {
        self.anim
            .iter()
            .any(|b| b.iter().any(SpringValue::is_animating))
    }

    /// Put every value where it belongs, instantly (tests, snapshots).
    pub fn settle(&mut self) {
        for baris in &mut self.anim {
            for v in baris.iter_mut() {
                v.settle();
            }
        }
    }

    // -- layout ------------------------------------------------------------

    /// Recompute the geometry and re-shape every label.
    ///
    /// The two-pass structure is the answer to the circular dependency
    /// described in the module docs: pass one builds the geometry against the
    /// **whole** content rect purely to find out how long the tick labels turn
    /// out to be; pass two rebuilds it against the rect that is left once those
    /// labels have been given their room. Two passes converge because the only
    /// thing that changes between them is the tick *count*, and a tick count
    /// does not change a label's length by more than a digit.
    fn rebuild(&mut self, size: Size) {
        self.labels.clear();
        self.swatches.clear();
        self.geometry = None;
        self.built_for = size;
        self.built_scale = self.fonts.scale_factor();
        self.stale = false;

        let pad = self.style.padding;
        let mut content = Rect::new(
            pad,
            pad,
            (size.width - pad * 2.0).max(0.0),
            (size.height - pad * 2.0).max(0.0),
        );
        if content.size.width <= 1.0 || content.size.height <= 1.0 {
            return;
        }

        // -- title ---------------------------------------------------------
        if let Some(judul) = self.spec.title.clone() {
            let gaya = self.style.title_text.clone();
            let warna = self.style.label;
            let ukuran = self.measure(&judul, &gaya);
            let asal = Point::new(content.min_x(), content.min_y());
            self.push_label(&judul, asal, warna, &gaya);
            content = shrink_top(content, ukuran.height + self.style.title_gap);
        }

        // -- empty state ---------------------------------------------------
        if self.data.is_empty() {
            let pesan = self.spec.empty_message.clone();
            let gaya = self.style.empty_text.clone();
            let warna = self.style.empty_label;
            let ukuran = self.measure(&pesan, &gaya);
            let asal = Point::new(
                content.min_x() + (content.size.width - ukuran.width) * 0.5,
                content.min_y() + (content.size.height - ukuran.height) * 0.5,
            );
            self.push_label(&pesan, asal, warna, &gaya);
            return;
        }

        // -- legend --------------------------------------------------------
        if self.spec.legend && self.data.series.len() > 1 {
            let tinggi = self.layout_legend(content);
            content = shrink_top(content, tinggi + self.style.legend_gap);
        }
        if content.size.width <= 1.0 || content.size.height <= 1.0 {
            return;
        }

        // -- pass one: how much room do the labels want? -------------------
        let sementara = PlotGeometry::build(content, &self.spec, &self.data);
        let gaya_tick = self.style.tick_text.clone();
        let lebar_nilai = if self.spec.value_axis {
            sementara
                .value_ticks
                .iter()
                .map(|t| self.measure(&t.label, &gaya_tick).width)
                .fold(0.0f32, f32::max)
        } else {
            0.0
        };
        let tinggi_nilai = if self.spec.value_axis {
            sementara
                .value_ticks
                .first()
                .map(|t| self.measure(&t.label, &gaya_tick).height)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let (lebar_kategori, tinggi_kategori) = if self.spec.category_axis {
            let mut w = 0.0f32;
            let mut h = 0.0f32;
            for t in &sementara.category_ticks {
                let s = self.measure(&t.label, &gaya_tick);
                w = w.max(s.width);
                h = h.max(s.height);
            }
            (w, h)
        } else {
            (0.0, 0.0)
        };

        // -- pass two: the plot rect that is actually left ------------------
        let gap = self.style.tick_gap;
        let plot = match self.spec.orientation {
            Orientation::Vertical => content.deflate(Insets {
                // Half a value label pokes above the topmost gridline, and half
                // a category label past each end of the axis; reserving both
                // keeps the outermost labels inside the node's own box.
                top: tinggi_nilai * 0.5,
                right: (lebar_kategori * 0.5).min(content.size.width * 0.15),
                bottom: if tinggi_kategori > 0.0 {
                    tinggi_kategori + gap
                } else {
                    0.0
                },
                left: if lebar_nilai > 0.0 {
                    lebar_nilai + gap
                } else {
                    0.0
                },
            }),
            Orientation::Horizontal => content.deflate(Insets {
                top: 0.0,
                right: (lebar_nilai * 0.5).min(content.size.width * 0.15),
                bottom: if tinggi_nilai > 0.0 {
                    tinggi_nilai + gap
                } else {
                    0.0
                },
                left: if lebar_kategori > 0.0 {
                    lebar_kategori + gap
                } else {
                    0.0
                },
            }),
        };
        if plot.size.width <= 1.0 || plot.size.height <= 1.0 {
            return;
        }

        let geometry = PlotGeometry::build(plot, &self.spec, &self.data);
        self.place_tick_labels(&geometry, &gaya_tick);
        self.geometry = Some(geometry);
    }

    /// Lay the legend out in a row and return its height.
    fn layout_legend(&mut self, content: Rect) -> f32 {
        let gaya = self.style.legend_text.clone();
        let swatch = self.style.swatch_size;
        let gap_swatch = self.style.swatch_gap;
        let gap_entri = self.style.legend_entry_gap;
        let warna_teks = self.style.label;

        let entri: Vec<(String, Color)> = self
            .data
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), self.style.series_color(i, s.color)))
            .collect();

        let mut x = content.min_x();
        let mut tinggi = swatch;
        for (nama, warna) in entri {
            let ukuran = self.measure(&nama, &gaya);
            let lebar = swatch + gap_swatch + ukuran.width;
            if x > content.min_x() && x + lebar > content.max_x() {
                // One row only: a legend that wrapped would eat the plot it is
                // explaining. What does not fit is dropped, and the tooltip
                // still names every series.
                break;
            }
            let baris_y = content.min_y();
            tinggi = tinggi.max(ukuran.height);
            self.swatches.push(Swatch {
                rect: Rect::new(
                    x,
                    baris_y + (ukuran.height - swatch).max(0.0) * 0.5,
                    swatch,
                    swatch,
                ),
                color: warna,
            });
            self.push_label(
                &nama,
                Point::new(x + swatch + gap_swatch, baris_y),
                warna_teks,
                &gaya,
            );
            x += lebar + gap_entri;
        }
        tinggi
    }

    /// Place the tick labels around the plot.
    fn place_tick_labels(&mut self, geometry: &PlotGeometry, gaya: &TextStyle) {
        let gap = self.style.tick_gap;
        let plot = geometry.plot;
        let warna = self.style.tick_label;

        if self.spec.value_axis {
            for t in geometry.value_ticks.clone() {
                let ukuran = self.measure(&t.label, gaya);
                let asal = match self.spec.orientation {
                    // Right-aligned against the plot: a column of numbers is
                    // read by its last digit, not its first.
                    Orientation::Vertical => Point::new(
                        plot.min_x() - gap - ukuran.width,
                        t.position - ukuran.height * 0.5,
                    ),
                    Orientation::Horizontal => {
                        Point::new(t.position - ukuran.width * 0.5, plot.max_y() + gap)
                    }
                };
                self.push_label(&t.label, asal, warna, gaya);
            }
        }

        if self.spec.category_axis {
            for t in geometry.category_ticks.clone() {
                let ukuran = self.measure(&t.label, gaya);
                let asal = match self.spec.orientation {
                    Orientation::Vertical => {
                        Point::new(t.position - ukuran.width * 0.5, plot.max_y() + gap)
                    }
                    Orientation::Horizontal => Point::new(
                        plot.min_x() - gap - ukuran.width,
                        t.position - ukuran.height * 0.5,
                    ),
                };
                self.push_label(&t.label, asal, warna, gaya);
            }
        }
    }

    fn measure(&self, text: &str, style: &TextStyle) -> Size {
        self.fonts.with(|m| m.measure_line(text, style))
    }

    fn push_label(&mut self, text: &str, origin: Point, color: Color, style: &TextStyle) {
        if text.is_empty() {
            return;
        }
        let run = self.fonts.with(|m| {
            let layout = m.layout(text, style, TextConstraints::UNBOUNDED);
            m.rasterize(&layout, origin, color)
        });
        if !run.is_empty() {
            self.labels.push(Placed { run });
        }
    }

    // -- paint -------------------------------------------------------------

    fn paint_marks(&self, ctx: &mut PaintCtx<'_>, geometry: &PlotGeometry) {
        match self.spec.kind {
            ChartKind::Bar => self.paint_bars(ctx, geometry),
            ChartKind::Area => {
                self.paint_areas(ctx, geometry);
                self.paint_lines(ctx, geometry);
            }
            ChartKind::Line | ChartKind::Sparkline => self.paint_lines(ctx, geometry),
        }
    }

    /// The screen points of one series, skipping the gaps.
    ///
    /// Returns **runs** rather than one list: a missing measurement must break
    /// the line, not be bridged by a straight segment that no data supports.
    fn series_points(&self, series: usize, geometry: &PlotGeometry) -> Vec<Vec<Point>> {
        let mut runs: Vec<Vec<Point>> = Vec::new();
        let mut current: Vec<Point> = Vec::new();
        for i in 0..self.data.len() {
            match self.drawn_value(series, i) {
                Some(v) => current.push(geometry.point(i, self.data.x[i], v)),
                None => {
                    if !current.is_empty() {
                        runs.push(std::mem::take(&mut current));
                    }
                }
            }
        }
        if !current.is_empty() {
            runs.push(current);
        }
        runs
    }

    fn paint_lines(&self, ctx: &mut PaintCtx<'_>, geometry: &PlotGeometry) {
        let plot = geometry.plot;
        let bulat = |w: f32| Corners::uniform(w * 0.5, self.style.bar_corners.style);
        for (si, series) in self.data.series.iter().enumerate() {
            let warna = self.style.series_color(si, series.color);
            let width = self.style.line_width;
            for titik in self.series_points(si, geometry) {
                // ONE stroke command for the whole run. This used to be one
                // vertical box per pixel column, with its height corrected by
                // √(1+m²) so steep segments did not thin out; a real stroke has
                // the right thickness by construction, and round joins mean two
                // segments meet without a notch.
                if titik.len() >= 2 {
                    let mut garis = Stroke::with_capacity(warna, width, titik.len())
                        .cap(LineCap::Round)
                        .join(LineJoin::Round)
                        .clip(plot);
                    garis.extend(titik.iter().copied());
                    ctx.stroke(garis);
                } else if let Some(p) = titik.first() {
                    // A run of one point is a dot, not a line — a single reading
                    // between two gaps still has to be visible.
                    if plot.contains(*p) {
                        ctx.quad(
                            Quad::new(stroke::marker_rect(*p, width))
                                .background(warna)
                                .corners(bulat(width)),
                        );
                    }
                }
                if self.spec.markers {
                    for p in &titik {
                        if !plot.contains(*p) {
                            continue;
                        }
                        let r = stroke::marker_rect(*p, self.style.marker_size);
                        ctx.quad(
                            Quad::new(r)
                                .background(warna)
                                .corners(bulat(self.style.marker_size))
                                // A ring in the page's own color, so two series
                                // crossing stay two lines rather than a blob.
                                .border(self.style.hover_ring, self.style.segment_gap_color),
                        );
                    }
                }
            }
        }
    }

    fn paint_areas(&self, ctx: &mut PaintCtx<'_>, geometry: &PlotGeometry) {
        let plot = geometry.plot;
        for (si, series) in self.data.series.iter().enumerate() {
            let warna = self
                .style
                .palette
                .fill(self.style.series_color(si, series.color));
            for titik in self.series_points(si, geometry) {
                for r in stroke::area_columns(&titik, geometry.baseline, COLUMN_STEP) {
                    if let Some(r) = r.intersect(plot) {
                        ctx.quad(Quad::new(r).background(warna));
                    }
                }
            }
        }
    }

    fn paint_bars(&self, ctx: &mut PaintCtx<'_>, geometry: &PlotGeometry) {
        let plot = geometry.plot;
        let stacked = self.spec.is_stacked();
        let jumlah = self.data.series.len().max(1);
        let lebar_band = match &geometry.category {
            CategoryScale::Band(b) => b.band_width(),
            other => other.band_width(self.data.len()),
        };

        for (si, series) in self.data.series.iter().enumerate() {
            let warna = self.style.series_color(si, series.color);
            for i in 0..self.data.len() {
                let Some(nilai) = self.drawn_value(si, i) else {
                    continue;
                };
                let (center, lebar) = if stacked {
                    (geometry.category.position(i, self.data.x[i]), lebar_band)
                } else {
                    let sub = lebar_band / jumlah as f32;
                    let mulai = geometry.category.position(i, self.data.x[i]) - lebar_band * 0.5;
                    (mulai + sub * (si as f32 + 0.5), sub)
                };
                let dasar = if stacked {
                    self.data.stack_base(si, i)
                } else {
                    0.0
                };
                let mut r = geometry.bar_rect(center, lebar, dasar, dasar + nilai);
                if stacked && si > 0 {
                    // A 2pt gap of the page's own color between segments — the
                    // eye reads separated fills as separate quantities, and
                    // adjacent fills of similar hue as one.
                    r = inset_along_value(r, geometry.orientation, self.style.segment_gap);
                }
                if let Some(r) = r.intersect(plot) {
                    ctx.quad(
                        Quad::new(r)
                            .background(warna)
                            .corners(self.style.bar_corners.clamp_to(r.size)),
                    );
                }
            }
        }
    }

    fn paint_crosshair(&self, ctx: &mut PaintCtx<'_>, geometry: &PlotGeometry) {
        let Some(index) = self.hover else {
            return;
        };
        if index >= self.data.len() {
            return;
        }
        let posisi = geometry.category.position(index, self.data.x[index]);
        let r = geometry.category_gridline(posisi, self.style.hairline);
        if let Some(r) = r.intersect(geometry.plot) {
            ctx.quad(Quad::new(r).background(self.style.crosshair));
        }
    }

    // -- accessibility -----------------------------------------------------

    /// The sentence a screen reader announces.
    ///
    /// A chart is a picture, and the accessible form of a picture is a
    /// description — so this says what the marks say: how many series, over
    /// what range, from what to what. Without it a screen reader announces
    /// "image" and the reader learns nothing (§3.8, and failure mode #2: the
    /// retrofit that never happens).
    pub fn summary(&self) -> String {
        if self.data.is_empty() {
            return self.spec.empty_message.clone();
        }
        let jenis = match self.spec.kind {
            ChartKind::Line => "line chart",
            ChartKind::Area => "area chart",
            ChartKind::Bar => "bar chart",
            ChartKind::Sparkline => "sparkline",
        };
        let (lo, hi) = self.data.value_domain(self.spec.is_stacked());
        let f = &self.spec.value_format;
        let l = &self.spec.locale;
        let mut out = format!(
            "{jenis}, {} series, {} points, {} to {}",
            self.data.series.len(),
            self.data.len(),
            f.format(lo, l),
            f.format(hi, l)
        );
        if self.data.series.len() > 1 {
            out.push_str(": ");
            let nama: Vec<&str> = self.data.series.iter().map(|s| s.name.as_str()).collect();
            out.push_str(&nama.join(", "));
        }
        out
    }

    /// Build the hover payload for one point, in global coordinates.
    fn hover_payload(
        &self,
        index: usize,
        node_origin: Point,
        geometry: &PlotGeometry,
    ) -> ChartHover {
        let entries = self
            .data
            .series
            .iter()
            .enumerate()
            .filter_map(|(si, s)| {
                let v = s.value(index)?;
                Some(HoverEntry {
                    series: si,
                    name: s.name.clone(),
                    value: v,
                    text: self.spec.value_format.format(v, &self.spec.locale),
                    color: self.style.series_color(si, s.color),
                })
            })
            .collect();
        let posisi = geometry.category.position(
            index,
            self.data.x.get(index).copied().unwrap_or(index as f64),
        );
        // The anchor is the whole column at the hovered position: anchoring to
        // a single mark would make the panel jump up and down as the reader
        // sweeps across the chart.
        let lokal = match geometry.orientation {
            Orientation::Vertical => Rect::new(
                posisi - 1.0,
                geometry.plot.min_y(),
                2.0,
                geometry.plot.size.height,
            ),
            Orientation::Horizontal => Rect::new(
                geometry.plot.min_x(),
                posisi - 1.0,
                geometry.plot.size.width,
                2.0,
            ),
        };
        ChartHover {
            index,
            title: self
                .data
                .label(index, &self.spec.locale, &self.spec.category_format),
            entries,
            anchor: lokal.translated(node_origin),
        }
    }

    fn set_hover(&mut self, index: Option<usize>, ctx: &mut EventCtx<'_>) {
        if self.hover == index {
            return;
        }
        self.hover = index;
        ctx.request_paint();
        let Some(f) = self.on_hover.clone() else {
            return;
        };
        let payload = match (index, self.geometry.as_ref()) {
            (Some(i), Some(g)) => {
                let asal = ctx.bounds().origin;
                Some(self.hover_payload(i, asal, g))
            }
            _ => None,
        };
        f(payload);
    }
}

impl std::fmt::Debug for ChartBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartBox")
            .field("kind", &self.spec.kind)
            .field("series", &self.data.series.len())
            .field("points", &self.data.len())
            .field("size", &self.size)
            .finish()
    }
}

impl RenderNode for ChartBox {
    fn type_name(&self) -> &'static str {
        "Chart"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let ukuran = constraints.constrain(preferred_size(
            Size::new(constraints.max_width, constraints.max_height),
            self.spec.kind,
        ));
        self.size = ukuran;
        let skala = self.fonts.scale_factor();
        if self.stale || self.built_for != ukuran || self.built_scale != skala {
            self.rebuild(ukuran);
        }
        ukuran
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let Some(geometry) = self.geometry.as_ref() else {
            // No geometry means either "no room" or "no data"; in the latter
            // case the empty-state label is already in `labels`.
            for l in &self.labels {
                ctx.glyph_run(l.run.clone());
            }
            return;
        };
        let plot = geometry.plot;

        if self.style.plot_background.a > 0.0 {
            ctx.quad(
                Quad::new(plot)
                    .background(self.style.plot_background)
                    .corners(self.style.plot_corners.clamp_to(plot.size)),
            );
        }

        // Gridlines: recessive by construction — the `separator` token, one
        // hairline thick. They are scaffolding, not data.
        if self.spec.grid {
            for t in &geometry.value_ticks {
                let r = geometry.value_gridline(t.position, self.style.hairline);
                if let Some(r) = r.intersect(plot) {
                    ctx.quad(Quad::new(r).background(self.style.grid));
                }
            }
        }

        // The zero rule is stronger than a gridline: "no change" is a landmark.
        if geometry.value.contains(0.0) {
            let r = geometry.value_gridline(geometry.baseline, self.style.hairline * 1.5);
            if let Some(r) = r.intersect(plot) {
                ctx.quad(Quad::new(r).background(self.style.zero_rule));
            }
        }

        self.paint_marks(ctx, geometry);
        self.paint_crosshair(ctx, geometry);

        // The axis rules, drawn after the marks so a bar sitting on the axis
        // does not swallow it.
        if self.spec.category_axis || self.spec.value_axis {
            let sumbu = match geometry.orientation {
                Orientation::Vertical => Rect::new(
                    plot.min_x(),
                    geometry.baseline.max(plot.min_y()).min(plot.max_y())
                        - self.style.hairline * 0.5,
                    plot.size.width,
                    self.style.hairline,
                ),
                Orientation::Horizontal => Rect::new(
                    geometry.baseline.max(plot.min_x()).min(plot.max_x())
                        - self.style.hairline * 0.5,
                    plot.min_y(),
                    self.style.hairline,
                    plot.size.height,
                ),
            };
            ctx.quad(Quad::new(sumbu).background(self.style.axis));
        }

        for s in &self.swatches {
            ctx.quad(
                Quad::new(s.rect)
                    .background(s.color)
                    .corners(Corners::uniform(
                        self.style.swatch_size * 0.25,
                        self.style.bar_corners.style,
                    )),
            );
        }
        for l in &self.labels {
            ctx.glyph_run(l.run.clone());
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // `Image` with a description, which is what an accessible chart is on
        // every platform: the picture is announced, and the description carries
        // what the picture says. A dedicated chart role would have to be added
        // to `silka-core`'s vocabulary *and* mapped in the platform adapter;
        // until then this is the honest answer rather than a silent
        // `Container`, which would make the chart invisible entirely.
        node.role = AccessRole::Image;
        node.label = Some(
            self.spec
                .title
                .clone()
                .unwrap_or_else(|| "Chart".to_string()),
        );
        node.value = Some(self.summary());
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Opaque: the chart wants every pointer move inside its box, because
        // hover is answered for the whole plot and not just for the marks.
        HitBehavior::Opaque
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else {
            return;
        };
        match p.phase {
            PointerPhase::Move | PointerPhase::Enter | PointerPhase::Down => {
                let lokal = ctx.local();
                let index = self
                    .geometry
                    .as_ref()
                    .filter(|g| g.plot.contains(lokal))
                    .and_then(|g| g.index_at(lokal, &self.data));
                self.set_hover(index, ctx);
            }
            PointerPhase::Leave | PointerPhase::Cancel => self.set_hover(None, ctx),
            _ => {}
        }
    }
}

/// The magnitude one spring unit stands for: the largest absolute value in the
/// data, never below one.
///
/// The floor matters as much as the maximum. A chart whose values are all in the
/// thousandths would otherwise get a scale near zero, and dividing by it turns a
/// modest dataset into astronomically large spring positions — the same failure
/// as the one this normalisation exists to fix, reached from the other end.
fn value_scale(data: &ChartData) -> f64 {
    let mut skala = 0.0f64;
    for s in &data.series {
        for v in s.values.iter().filter(|v| v.is_finite()) {
            skala = skala.max(v.abs());
        }
    }
    if skala.is_finite() && skala > 1.0 {
        skala
    } else {
        1.0
    }
}

/// Shrink a rect from the top by `amount`.
fn shrink_top(rect: Rect, amount: f32) -> Rect {
    Rect::new(
        rect.origin.x,
        rect.origin.y + amount,
        rect.size.width,
        (rect.size.height - amount).max(0.0),
    )
}

/// Take `gap` off a bar's **base** end, along the value axis.
///
/// Which end that is depends on which side of the baseline the bar is on, and
/// getting it wrong makes the gap appear at the far tip — where it shortens the
/// bar and therefore misstates the value.
fn inset_along_value(rect: Rect, orientation: Orientation, gap: f32) -> Rect {
    match orientation {
        Orientation::Vertical => {
            let h = (rect.size.height - gap).max(1.0);
            Rect::new(rect.origin.x, rect.origin.y, rect.size.width, h)
        }
        Orientation::Horizontal => {
            let w = (rect.size.width - gap).max(1.0);
            Rect::new(rect.origin.x + gap, rect.origin.y, w, rect.size.height)
        }
    }
}

/// Advance every chart in a tree by one frame — see [`crate::advance`].
pub(crate) fn walk(tree: &silka_core::tree::RenderTree) -> Vec<silka_core::tree::NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(
    tree: &silka_core::tree::RenderTree,
    id: silka_core::tree::NodeId,
    out: &mut Vec<silka_core::tree::NodeId>,
) {
    out.push(id);
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}
