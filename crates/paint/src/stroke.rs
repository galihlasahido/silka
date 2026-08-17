//! Real lines: a **stroke** command with width, caps, and joins.
//!
//! Until this existed, every line in the framework was a fake. A chart series
//! was rasterised into one vertical box per column of pixels
//! (`silka_chart::stroke`), and a checkbox tick was a round pen stamped a dozen
//! times along a path (`silka_widgets::check_dots`). Both were honest about being
//! debt, and both produced dozens of commands for what is geometrically a single
//! shape.
//!
//! A [`Stroke`] carries the whole polyline instead, and the backend rasterises it
//! from a signed distance field: one instance per segment, anti-aliased from
//! screen-space derivatives, correct on any DPI, and — because the geometry is a
//! capsule rather than a stack of boxes — actually the shape a designer drew.
//!
//! ```
//! use silka_paint::{Color, LineCap, LineJoin, Point, Stroke};
//!
//! // A chart series: as many points as the data has, one command.
//! let mut line = Stroke::new(Color::hex(0x0A84FF), 2.0)
//!     .cap(LineCap::Round)
//!     .join(LineJoin::Round);
//! for (i, y) in [40.0, 22.0, 31.0, 12.0].into_iter().enumerate() {
//!     line.push(Point::new(i as f32 * 20.0, y));
//! }
//!
//! assert_eq!(line.segment_count(), 3);
//! assert!(line.is_visible());
//!
//! // Its bounds include half the stroke width on every side, so a dirty region
//! // computed from them never clips the line's own edge.
//! let b = line.bounds().unwrap();
//! assert!(b.min_x() <= -1.0 && b.max_x() >= 61.0);
//! ```
//!
//! ## What the geometry means
//!
//! `width` is the **full** perpendicular width, the way every drawing API means
//! it: a 2pt stroke covers 1pt on each side of the path. Caps and joins keep
//! their usual meanings, and the two that need explaining:
//!
//! - [`LineCap::Square`] is [`LineCap::Butt`] with the path extended by half the
//!   width at each end, which is how it is defined — and doing that extension on
//!   the CPU ([`LineCap::extension`]) is why the shader needs no cap branch.
//! - [`LineJoin::Miter`] is bounded by [`Stroke::miter_limit`], because an almost
//!   doubled-back path produces a spike that would otherwise run off screen. A
//!   spike past the limit degrades to [`LineJoin::Bevel`], which is what SVG and
//!   Core Graphics both do.

use crate::color::Color;
use crate::geometry::{Point, Rect};

/// How a stroke ends.
///
/// ```
/// use silka_paint::LineCap;
///
/// // Butt stops dead on the last point; square reaches half a width past it.
/// assert_eq!(LineCap::Butt.extension(4.0), 0.0);
/// assert_eq!(LineCap::Square.extension(4.0), 2.0);
/// // A round cap covers the same ground as square but as a semicircle, so the
/// // path is not extended — the SDF's own radius does the work.
/// assert_eq!(LineCap::Round.extension(4.0), 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    /// Stops exactly at the endpoint.
    #[default]
    Butt,
    /// A semicircle of radius `width / 2` beyond the endpoint.
    Round,
    /// A square half a width beyond the endpoint.
    Square,
}

impl LineCap {
    /// How far past its endpoint this cap extends the path, in logical points.
    ///
    /// A [`LineCap::Round`] cap answers zero on purpose: it is drawn by the
    /// segment's own distance field, not by moving the endpoint.
    pub fn extension(self, width: f32) -> f32 {
        match self {
            LineCap::Butt | LineCap::Round => 0.0,
            LineCap::Square => width.max(0.0) * 0.5,
        }
    }

    /// True when the segment's distance field should round off at the ends.
    pub fn is_round(self) -> bool {
        matches!(self, LineCap::Round)
    }
}

/// How two segments meet at an interior vertex.
///
/// ```
/// use silka_paint::LineJoin;
///
/// // Round is the safe default for data lines: no spikes, no seams.
/// assert_eq!(LineJoin::default(), LineJoin::Round);
/// assert!(LineJoin::Round.needs_vertex_dot());
/// // A mitre is drawn by the segments themselves once they are wide enough to
/// // overlap; only the round join needs a dot at the vertex.
/// assert!(!LineJoin::Miter.needs_vertex_dot());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    /// A sharp corner, bounded by [`Stroke::miter_limit`].
    Miter,
    /// A circular arc filling the wedge — the default, because it never spikes.
    #[default]
    Round,
    /// The wedge cut off with a straight edge.
    Bevel,
}

impl LineJoin {
    /// True when the backend fills the wedge with a round dot at the vertex.
    pub fn needs_vertex_dot(self) -> bool {
        matches!(self, LineJoin::Round)
    }
}

/// The default mitre limit — the SVG and PostScript value.
pub const DEFAULT_MITER_LIMIT: f32 = 4.0;

/// A stroked polyline: the whole path in one command.
///
/// Coordinates are logical points, in the same space as every other command: the
/// paint pass has already lifted them out of the node's local space.
///
/// ```
/// use silka_paint::{Color, LineCap, Point, Stroke};
///
/// // The two-point case has its own constructor because it is so common:
/// // separators, chart rules, tick marks.
/// let rule = Stroke::line(
///     Point::new(0.0, 0.5),
///     Point::new(320.0, 0.5),
///     Color::WHITE.with_alpha(0.12),
///     1.0,
/// );
/// assert_eq!(rule.segment_count(), 1);
/// assert_eq!(rule.half_width(), 0.5);
///
/// // A stroke with no width, no colour, or fewer than two points draws nothing
/// // — so "no data yet" costs nothing.
/// assert!(!Stroke::new(Color::WHITE, 0.0).is_visible());
/// assert!(!Stroke::new(Color::WHITE, 2.0).is_visible());
/// assert!(!rule.clone().cap(LineCap::Square).points.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    /// The path vertices, in order.
    pub points: Vec<Point>,
    /// The full perpendicular width, in logical points.
    pub width: f32,
    /// The stroke colour — a resolved theme token, like every other colour that
    /// reaches this layer.
    pub color: Color,
    /// How the two ends are finished.
    pub cap: LineCap,
    /// How interior vertices are finished.
    pub join: LineJoin,
    /// Whether the last point joins back to the first.
    pub closed: bool,
    /// The mitre spike limit, as a multiple of the stroke width.
    pub miter_limit: f32,
    /// An optional clip in the same coordinate space, resolved on the CPU.
    ///
    /// Used the same way [`crate::GlyphRun::clip`] is: a series inside a scroll
    /// viewport is cut here rather than costing the backend a scissor change.
    pub clip: Option<Rect>,
}

impl Stroke {
    /// An empty stroke of a given colour and width.
    pub fn new(color: Color, width: f32) -> Self {
        Self {
            points: Vec::new(),
            width: width.max(0.0),
            color,
            cap: LineCap::default(),
            join: LineJoin::default(),
            closed: false,
            miter_limit: DEFAULT_MITER_LIMIT,
            clip: None,
        }
    }

    /// An empty stroke with room for `capacity` points reserved up front.
    pub fn with_capacity(color: Color, width: f32, capacity: usize) -> Self {
        let mut s = Self::new(color, width);
        s.points.reserve(capacity);
        s
    }

    /// A single straight segment.
    pub fn line(from: Point, to: Point, color: Color, width: f32) -> Self {
        let mut s = Self::with_capacity(color, width, 2);
        s.points.push(from);
        s.points.push(to);
        s
    }

    /// A closed rectangle outline.
    ///
    /// Not the same thing as a bordered [`crate::Quad`]: a quad's border is inset
    /// into the box, while this stroke is centred on the rect's edge. Focus rings
    /// that must sit *outside* their control want this one.
    pub fn rect(rect: Rect, color: Color, width: f32) -> Self {
        let mut s = Self::with_capacity(color, width, 4);
        s.points.push(Point::new(rect.min_x(), rect.min_y()));
        s.points.push(Point::new(rect.max_x(), rect.min_y()));
        s.points.push(Point::new(rect.max_x(), rect.max_y()));
        s.points.push(Point::new(rect.min_x(), rect.max_y()));
        s.closed = true;
        s
    }

    /// Append one point.
    pub fn push(&mut self, p: Point) -> &mut Self {
        self.points.push(p);
        self
    }

    /// Append many points.
    pub fn extend(&mut self, points: impl IntoIterator<Item = Point>) -> &mut Self {
        self.points.extend(points);
        self
    }

    /// Set the cap style.
    pub fn cap(mut self, cap: LineCap) -> Self {
        self.cap = cap;
        self
    }

    /// Set the join style.
    pub fn join(mut self, join: LineJoin) -> Self {
        self.join = join;
        self
    }

    /// Close the path (the last point joins back to the first).
    pub fn closed(mut self, closed: bool) -> Self {
        self.closed = closed;
        self
    }

    /// Set the mitre limit.
    pub fn miter_limit(mut self, limit: f32) -> Self {
        self.miter_limit = limit.max(1.0);
        self
    }

    /// Set the CPU-resolved clip.
    pub fn clip(mut self, rect: Rect) -> Self {
        self.clip = Some(rect);
        self
    }

    /// Half the stroke width — the SDF radius.
    pub fn half_width(&self) -> f32 {
        self.width.max(0.0) * 0.5
    }

    /// The number of segments this path has, closing segment included.
    pub fn segment_count(&self) -> usize {
        match self.points.len() {
            0 | 1 => 0,
            n if self.closed => n,
            n => n - 1,
        }
    }

    /// The segments in order, as `(from, to)` pairs.
    ///
    /// The closing segment is produced last when [`Stroke::closed`] is set, so a
    /// backend never has to special-case it.
    pub fn segments(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        let n = self.points.len();
        let closing = self.closed && n > 2;
        self.points
            .windows(2)
            .map(|w| (w[0], w[1]))
            .chain(closing.then(|| (self.points[n - 1], self.points[0])))
    }

    /// True when this stroke can contribute any pixels at all.
    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.color.a > 0.0 && self.segment_count() > 0
    }

    /// The bounding box, half a stroke width proud on every side.
    ///
    /// `None` for a path that draws nothing. Mitre spikes are covered too: the
    /// limit is what bounds them, which is the whole reason a limit exists.
    pub fn bounds(&self) -> Option<Rect> {
        if self.points.is_empty() {
            return None;
        }
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &self.points {
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        if !min_x.is_finite() || !min_y.is_finite() {
            return None;
        }
        let margin = match self.join {
            LineJoin::Miter => self.half_width() * self.miter_limit.max(1.0),
            LineJoin::Round | LineJoin::Bevel => self.half_width(),
        } + self.cap.extension(self.width);
        Some(Rect::new(
            min_x - margin,
            min_y - margin,
            (max_x - min_x) + margin * 2.0,
            (max_y - min_y) + margin * 2.0,
        ))
    }

    /// A copy shifted by `offset`.
    ///
    /// This is how the paint pass lifts a node's local path into absolute window
    /// coordinates — a node never knows its own position.
    pub fn translated(&self, offset: Point) -> Stroke {
        Stroke {
            points: self
                .points
                .iter()
                .map(|p| Point::new(p.x + offset.x, p.y + offset.y))
                .collect(),
            clip: self.clip.map(|c| c.translated(offset)),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zigzag() -> Stroke {
        let mut s = Stroke::new(Color::WHITE, 2.0);
        s.extend([
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(20.0, 0.0),
        ]);
        s
    }

    #[test]
    fn lebar_dan_setengah_lebar() {
        let s = Stroke::new(Color::WHITE, 3.0);
        assert_eq!(s.width, 3.0);
        assert_eq!(s.half_width(), 1.5);
        // A negative width is clamped rather than inverting the geometry.
        assert_eq!(Stroke::new(Color::WHITE, -4.0).width, 0.0);
    }

    #[test]
    fn jumlah_ruas_terbuka_dan_tertutup() {
        assert_eq!(zigzag().segment_count(), 2);
        assert_eq!(zigzag().closed(true).segment_count(), 3);
        assert_eq!(Stroke::new(Color::WHITE, 1.0).segment_count(), 0);
        let mut satu = Stroke::new(Color::WHITE, 1.0);
        satu.push(Point::ZERO);
        assert_eq!(satu.segment_count(), 0, "satu titik bukan garis");
        assert_eq!(satu.closed(true).segment_count(), 0);
    }

    #[test]
    fn ruas_tertutup_kembali_ke_titik_awal() {
        let s = zigzag().closed(true);
        let ruas: Vec<(Point, Point)> = s.segments().collect();
        assert_eq!(ruas.len(), 3);
        assert_eq!(ruas[2].0, Point::new(20.0, 0.0));
        assert_eq!(ruas[2].1, Point::new(0.0, 0.0));
    }

    #[test]
    fn rect_menghasilkan_empat_ruas_tertutup() {
        let s = Stroke::rect(Rect::new(0.0, 0.0, 10.0, 4.0), Color::WHITE, 1.0);
        assert!(s.closed);
        assert_eq!(s.segment_count(), 4);
        assert_eq!(s.segments().count(), 4);
    }

    #[test]
    fn tidak_terlihat_kalau_tanpa_lebar_warna_atau_titik() {
        assert!(!Stroke::new(Color::WHITE, 0.0).is_visible());
        assert!(zigzag().is_visible(), "zigzag harus terlihat");
        let mut transparan = zigzag();
        transparan.color = Color::TRANSPARENT;
        assert!(!transparan.is_visible());
        let mut tanpa_lebar = zigzag();
        tanpa_lebar.width = 0.0;
        assert!(!tanpa_lebar.is_visible());
    }

    #[test]
    fn bounds_menyertakan_setengah_lebar() {
        let s = Stroke::line(
            Point::new(0.0, 5.0),
            Point::new(100.0, 5.0),
            Color::WHITE,
            4.0,
        )
        .join(LineJoin::Round);
        let b = s.bounds().expect("ada titik");
        assert_eq!(b.min_x(), -2.0);
        assert_eq!(b.min_y(), 3.0);
        assert_eq!(b.max_x(), 102.0);
        assert_eq!(b.max_y(), 7.0);
    }

    #[test]
    fn bounds_miter_menyisakan_ruang_untuk_taji() {
        // A mitre spike reaches further than half a width, and the limit is
        // exactly what bounds it — otherwise a dirty region would cut the spike.
        let bulat = zigzag().join(LineJoin::Round).bounds().unwrap();
        let taji = zigzag().join(LineJoin::Miter).bounds().unwrap();
        assert!(taji.size.width > bulat.size.width);
    }

    #[test]
    fn bounds_cap_square_lebih_panjang() {
        let butt = Stroke::line(Point::ZERO, Point::new(10.0, 0.0), Color::WHITE, 4.0)
            .cap(LineCap::Butt)
            .bounds()
            .unwrap();
        let square = Stroke::line(Point::ZERO, Point::new(10.0, 0.0), Color::WHITE, 4.0)
            .cap(LineCap::Square)
            .bounds()
            .unwrap();
        assert!(square.size.width > butt.size.width);
    }

    #[test]
    fn titik_nan_tidak_meracuni_bounds() {
        // One bad datum must lose its own segment, not the whole series.
        let mut s = Stroke::new(Color::WHITE, 2.0);
        s.extend([
            Point::new(0.0, 0.0),
            Point::new(f32::NAN, 3.0),
            Point::new(10.0, 0.0),
        ]);
        let b = s.bounds().expect("masih ada titik waras");
        assert!(b.min_x().is_finite() && b.size.width.is_finite());
    }

    #[test]
    fn translated_menggeser_titik_dan_clip() {
        let s = zigzag().clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        let t = s.translated(Point::new(100.0, 50.0));
        assert_eq!(t.points[0], Point::new(100.0, 50.0));
        assert_eq!(t.clip.unwrap().origin, Point::new(100.0, 50.0));
        assert_eq!(t.width, s.width);
        assert_eq!(t.cap, s.cap);
    }

    #[test]
    fn cap_square_memperpanjang_jalur_setengah_lebar() {
        assert_eq!(LineCap::Square.extension(6.0), 3.0);
        assert_eq!(LineCap::Butt.extension(6.0), 0.0);
        assert_eq!(LineCap::Round.extension(6.0), 0.0);
        assert!(LineCap::Round.is_round());
        assert!(!LineCap::Square.is_round());
    }

    #[test]
    fn hanya_join_bulat_butuh_titik_simpul() {
        assert!(LineJoin::Round.needs_vertex_dot());
        assert!(!LineJoin::Miter.needs_vertex_dot());
        assert!(!LineJoin::Bevel.needs_vertex_dot());
    }

    #[test]
    fn miter_limit_tidak_pernah_di_bawah_satu() {
        assert_eq!(
            Stroke::new(Color::WHITE, 1.0).miter_limit(0.1).miter_limit,
            1.0
        );
        assert_eq!(
            Stroke::new(Color::WHITE, 1.0).miter_limit,
            DEFAULT_MITER_LIMIT
        );
    }
}
