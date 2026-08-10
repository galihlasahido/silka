//! **Turning a polyline into rounded boxes** — because the paint layer has no
//! stroke command yet.
//!
//! `silka-paint`'s vocabulary is deliberately small (§3.2): rounded rect,
//! shadow, glyph run, clip. That covers ~95% of a UI, and a chart is most of
//! the remaining 5% — a line chart is, unavoidably, a diagonal stroke.
//!
//! The precedent is already set in the widget catalogue:
//! [`check_dots`](silka_widgets::check_dots) draws a checkmark by stamping a
//! round pen along a path rather than waiting for a stroke primitive. This
//! module does the same job for a much longer path, and uses a different
//! technique for it, for a reason worth writing down:
//!
//! - **Stamping** (a round quad every few points along the path) costs one
//!   command per stamp. That is fine for a 12pt checkmark and ruinous for a
//!   600pt line: the same spacing would emit well over a thousand commands per
//!   series.
//! - **Column rasterisation** (this module) walks the polyline in **x**, and
//!   emits one vertical box per column of width [`COLUMN_STEP`]. The cost is
//!   bounded by the chart's *width*, not by its path length, and — the part
//!   that actually matters — identical neighbouring columns are merged, so the
//!   flat parts of a series cost one box no matter how long they are.
//!
//! The geometry is exact rather than approximate in the direction that counts:
//! a stroke of perpendicular width `w` on a slope `m` covers a **vertical**
//! extent of `w · √(1 + m²)`, and that is what [`stroke_columns`] emits. Emit
//! `w` flat, as the naive version does, and every steep segment of the line
//! silently thins out — which on a finance chart is exactly where the reader is
//! looking.
//!
//! Joins and caps are then a handful of round quads at the vertices
//! ([`joint_dots`]), which is where the cost of stamping is negligible: there
//! are as many vertices as data points, not as many as pixels.
//!
//! **This is acknowledged technical debt, not an accident.** The moment
//! `silka-paint` grows an SDF stroke command, [`stroke_columns`] collapses into
//! a single command and nothing outside this file changes — the same bargain
//! `check_dots` struck.
//!
//! ```
//! use silka_chart::stroke::{stroke_columns, COLUMN_STEP};
//! use silka_paint::Point;
//!
//! // A perfectly flat line costs exactly one box, however wide it is.
//! let datar = [Point::new(0.0, 50.0), Point::new(600.0, 50.0)];
//! assert_eq!(stroke_columns(&datar, 2.0, COLUMN_STEP).len(), 1);
//! ```

use silka_paint::{Point, Rect};

/// The width of one rasterisation column, in logical points.
///
/// One point is the resolution at which a reader can see a kink in a line;
/// finer columns buy nothing and cost commands. At 2× DPI this is still half a
/// physical pixel per column.
pub const COLUMN_STEP: f32 = 1.0;

/// How much a stroke is allowed to fatten vertically on a steep slope.
///
/// `w · √(1 + m²)` goes to infinity as a segment approaches vertical. Without
/// this ceiling a single near-vertical step in the data would paint a bar the
/// height of the plot — which is what a naive implementation looks like when
/// the data has a gap.
const MAX_SLOPE_FACTOR: f32 = 8.0;

/// The stroke of a polyline, as vertical boxes ready to become quads.
///
/// `points` are in the node's local coordinates and are expected to be ordered
/// in x (chart data is); segments that run backwards are still handled, they
/// simply produce their own columns.
pub fn stroke_columns(points: &[Point], width: f32, step: f32) -> Vec<Rect> {
    let width = width.max(0.1);
    let step = step.max(0.05);
    match points.len() {
        0 => return Vec::new(),
        1 => {
            let p = points[0];
            return vec![Rect::new(
                p.x - width * 0.5,
                p.y - width * 0.5,
                width,
                width,
            )];
        }
        _ => {}
    }

    let mut out: Vec<Rect> = Vec::with_capacity(64);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if !finite(a) || !finite(b) {
            continue;
        }
        let (a, b) = if a.x <= b.x { (a, b) } else { (b, a) };
        let dx = b.x - a.x;

        if dx <= step {
            // A vertical (or sub-column) segment: one box covering the whole
            // jump. Walking it in x would emit nothing at all.
            let top = a.y.min(b.y) - width * 0.5;
            let bottom = a.y.max(b.y) + width * 0.5;
            push_merged(
                &mut out,
                Rect::new(a.x - width * 0.5, top, width.max(dx), bottom - top),
            );
            continue;
        }

        let slope = (b.y - a.y) / dx;
        let vertical = (width * (1.0 + slope * slope).sqrt()).min(width * MAX_SLOPE_FACTOR);
        let mut x = a.x;
        while x < b.x {
            let xe = (x + step).min(b.x);
            let y0 = a.y + slope * (x - a.x);
            let y1 = a.y + slope * (xe - a.x);
            let top = y0.min(y1) - vertical * 0.5;
            let bottom = y0.max(y1) + vertical * 0.5;
            push_merged(&mut out, Rect::new(x, top, xe - x, bottom - top));
            x = xe;
        }
    }
    out
}

/// The fill between a polyline and a horizontal baseline — an area chart.
///
/// Shares the column walk with [`stroke_columns`] on purpose: an area whose
/// top edge disagreed with its own line by even half a point would show a
/// hairline of background between them, and chasing that later is far more
/// expensive than sharing the arithmetic now.
pub fn area_columns(points: &[Point], baseline: f32, step: f32) -> Vec<Rect> {
    let step = step.max(0.05);
    if points.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<Rect> = Vec::with_capacity(64);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if !finite(a) || !finite(b) {
            continue;
        }
        let (a, b) = if a.x <= b.x { (a, b) } else { (b, a) };
        let dx = b.x - a.x;
        if dx <= 0.0 {
            continue;
        }
        let slope = (b.y - a.y) / dx;
        let mut x = a.x;
        while x < b.x {
            let xe = (x + step).min(b.x);
            let y0 = a.y + slope * (x - a.x);
            let y1 = a.y + slope * (xe - a.x);
            let top = y0.min(y1).min(baseline);
            let bottom = y0.max(y1).max(baseline);
            if bottom > top {
                push_merged(&mut out, Rect::new(x, top, xe - x, bottom - top));
            }
            x = xe;
        }
    }
    out
}

/// Round caps and joins: one square (to be drawn fully rounded) per vertex.
///
/// Cheap precisely because there are as many vertices as data points. Returns
/// the boxes, not the points, so the caller can hand them straight to a quad
/// with `Corners::uniform(width / 2, …)`.
pub fn joint_dots(points: &[Point], width: f32) -> Vec<Rect> {
    let width = width.max(0.1);
    points
        .iter()
        .filter(|p| finite(**p))
        .map(|p| Rect::new(p.x - width * 0.5, p.y - width * 0.5, width, width))
        .collect()
}

/// A square box of side `size` centred on a point — a data marker.
pub fn marker_rect(center: Point, size: f32) -> Rect {
    let size = size.max(0.1);
    Rect::new(center.x - size * 0.5, center.y - size * 0.5, size, size)
}

/// Append a box, merging it into the previous one when they line up.
///
/// This is the optimisation that makes column rasterisation affordable: a flat
/// series, a plateau, a baseline — anything that does not change y — collapses
/// into a single command however wide it is.
fn push_merged(out: &mut Vec<Rect>, rect: Rect) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    if let Some(last) = out.last_mut() {
        let sama_y = (last.origin.y - rect.origin.y).abs() < 0.01
            && (last.size.height - rect.size.height).abs() < 0.01;
        let bersambung = (last.origin.x + last.size.width - rect.origin.x).abs() < 0.01;
        if sama_y && bersambung {
            last.size.width += rect.size.width;
            return;
        }
    }
    out.push(rect);
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total_width(rects: &[Rect]) -> f32 {
        rects.iter().map(|r| r.size.width).sum()
    }

    fn bounds(rects: &[Rect]) -> Rect {
        let min_x = rects
            .iter()
            .map(|r| r.min_x())
            .fold(f32::INFINITY, f32::min);
        let min_y = rects
            .iter()
            .map(|r| r.min_y())
            .fold(f32::INFINITY, f32::min);
        let max_x = rects
            .iter()
            .map(|r| r.max_x())
            .fold(f32::NEG_INFINITY, f32::max);
        let max_y = rects
            .iter()
            .map(|r| r.max_y())
            .fold(f32::NEG_INFINITY, f32::max);
        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    #[test]
    fn garis_datar_hanya_satu_kotak() {
        // The whole point of merging: a flat series must not cost 600 commands.
        let p = [Point::new(0.0, 50.0), Point::new(600.0, 50.0)];
        let r = stroke_columns(&p, 2.0, COLUMN_STEP);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].size.width, 600.0);
        assert!((r[0].size.height - 2.0).abs() < 1e-4);
        assert!((r[0].center().y - 50.0).abs() < 1e-4);
    }

    #[test]
    fn kolom_menutupi_seluruh_rentang_x_tanpa_celah() {
        // A gap between columns is a dotted line, and it is the kind of bug that
        // only shows up on a screenshot.
        let p = [
            Point::new(0.0, 10.0),
            Point::new(100.0, 90.0),
            Point::new(200.0, 30.0),
        ];
        let r = stroke_columns(&p, 2.0, COLUMN_STEP);
        assert!(
            (total_width(&r) - 200.0).abs() < 0.01,
            "{}",
            total_width(&r)
        );
        let b = bounds(&r);
        assert!((b.min_x() - 0.0).abs() < 0.01 && (b.max_x() - 200.0).abs() < 0.01);
        // Sorted and contiguous in x.
        for w in r.windows(2) {
            assert!(
                (w[0].max_x() - w[1].min_x()).abs() < 0.01,
                "{:?} {:?}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn ruas_curam_tidak_menipis() {
        // A stroke of perpendicular width w on a slope m covers w·√(1+m²)
        // vertically. Emitting a flat w instead makes every steep segment thin
        // out — precisely where a finance chart is being read.
        let datar = stroke_columns(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], 2.0, 1.0);
        let curam = stroke_columns(&[Point::new(0.0, 0.0), Point::new(100.0, 300.0)], 2.0, 1.0);
        let tinggi_datar = datar[0].size.height;
        let tinggi_curam = curam[0].size.height;
        assert!(
            tinggi_curam > tinggi_datar * 2.0,
            "datar {tinggi_datar} vs curam {tinggi_curam}"
        );
    }

    #[test]
    fn ruas_hampir_tegak_dibatasi() {
        // Without a ceiling, √(1+m²) explodes and one data gap paints a bar the
        // height of the plot.
        let r = stroke_columns(&[Point::new(0.0, 0.0), Point::new(2.0, 5_000.0)], 2.0, 1.0);
        let tertinggi = r.iter().map(|x| x.size.height).fold(0.0, f32::max);
        assert!(tertinggi <= 2.0 * MAX_SLOPE_FACTOR + 5_001.0);
        assert!(r.iter().all(|x| x.size.height.is_finite()));
    }

    #[test]
    fn ruas_tegak_sempurna_tetap_tergambar() {
        // dx = 0: walking in x emits nothing at all, so this needs its own path.
        let r = stroke_columns(&[Point::new(50.0, 10.0), Point::new(50.0, 90.0)], 3.0, 1.0);
        assert!(!r.is_empty());
        let b = bounds(&r);
        assert!(b.size.height >= 80.0, "{b:?}");
    }

    #[test]
    fn titik_tunggal_menjadi_satu_titik() {
        let r = stroke_columns(&[Point::new(10.0, 20.0)], 4.0, 1.0);
        assert_eq!(r.len(), 1);
        assert!((r[0].center().x - 10.0).abs() < 1e-4);
        assert!((r[0].center().y - 20.0).abs() < 1e-4);
        assert!(stroke_columns(&[], 4.0, 1.0).is_empty());
    }

    #[test]
    fn nilai_nan_dilewati_bukan_diambar() {
        // A NaN in the data must lose its segment, not poison the whole series.
        let p = [
            Point::new(0.0, 10.0),
            Point::new(50.0, f32::NAN),
            Point::new(100.0, 30.0),
        ];
        let r = stroke_columns(&p, 2.0, 1.0);
        assert!(r
            .iter()
            .all(|x| x.origin.y.is_finite() && x.size.height.is_finite()));
    }

    #[test]
    fn area_terisi_sampai_baseline() {
        let p = [Point::new(0.0, 20.0), Point::new(100.0, 20.0)];
        let r = area_columns(&p, 100.0, 1.0);
        assert_eq!(r.len(), 1, "atap datar harus jadi satu kotak");
        assert!((r[0].min_y() - 20.0).abs() < 0.01);
        assert!((r[0].max_y() - 100.0).abs() < 0.01);
    }

    #[test]
    fn area_melintasi_baseline_terisi_di_kedua_sisi() {
        // A series that goes negative: the fill has to appear above *and* below
        // the zero rule, otherwise the losses are invisible.
        let p = [Point::new(0.0, 0.0), Point::new(100.0, 200.0)];
        let r = area_columns(&p, 100.0, 1.0);
        assert!(
            r.iter().any(|x| x.min_y() < 100.0),
            "harus ada isi di atas baseline"
        );
        assert!(
            r.iter().any(|x| x.max_y() > 100.0),
            "harus ada isi di bawah baseline"
        );
    }

    #[test]
    fn area_butuh_dua_titik() {
        assert!(area_columns(&[Point::new(0.0, 0.0)], 10.0, 1.0).is_empty());
    }

    #[test]
    fn sambungan_satu_titik_per_simpul() {
        let p = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(20.0, 0.0),
        ];
        let d = joint_dots(&p, 4.0);
        assert_eq!(d.len(), 3);
        assert!((d[1].center().x - 10.0).abs() < 1e-4);
        assert_eq!(d[0].size.width, 4.0);
    }

    #[test]
    fn biaya_tumbuh_dengan_lebar_bukan_dengan_jumlah_titik() {
        // The claim the module is built on. A hundred points across 400pt must
        // not cost more than the columns that fit in 400pt.
        let titik: Vec<Point> = (0..100)
            .map(|i| Point::new(i as f32 * 4.0, if i % 2 == 0 { 10.0 } else { 90.0 }))
            .collect();
        let r = stroke_columns(&titik, 2.0, COLUMN_STEP);
        assert!(r.len() <= 420, "{} kotak untuk 400pt", r.len());
    }

    #[test]
    fn marker_terpusat_pada_titiknya() {
        let m = marker_rect(Point::new(30.0, 40.0), 6.0);
        assert!((m.center().x - 30.0).abs() < 1e-4);
        assert!((m.center().y - 40.0).abs() < 1e-4);
        assert_eq!(m.size.width, 6.0);
    }
}
