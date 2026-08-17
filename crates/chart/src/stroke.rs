//! **Area fills as columns**, and the small rects a chart needs besides.
//!
//! This module used to rasterise chart *lines* too: `silka-paint` had no stroke
//! command, so a diagonal was emitted as one vertical box per pixel column, with
//! its height corrected by `w · √(1 + m²)` so steep segments did not thin out.
//! That is gone. A line is now a real [`silka_paint::Stroke`] — one command for a
//! whole series, rasterised from a distance field, with the caps and joins a
//! designer actually asked for.
//!
//! What is left here is the part a stroke command does **not** answer:
//!
//! - [`area_columns`] — an area chart is a *fill* between a polyline and a
//!   baseline, not a stroke. Walking the polyline in x and emitting one merged
//!   column per step keeps the cost bounded by the chart's width rather than its
//!   path length, and — the part that matters — a flat stretch collapses into a
//!   single box however wide it is.
//! - [`marker_rect`] — a data marker is a small box (drawn fully rounded), and
//!   there are as many of them as there are data points.
//!
//! The column walk is shared with nothing now, but its arithmetic still has to
//! agree with the line drawn on top of it: an area whose top edge disagreed with
//! its own line by even half a point would show a hairline of background between
//! them.
//!
//! ```
//! use silka_chart::stroke::{area_columns, COLUMN_STEP};
//! use silka_paint::Point;
//!
//! // A flat top edge costs exactly one box, however wide it is.
//! let datar = [Point::new(0.0, 20.0), Point::new(600.0, 20.0)];
//! assert_eq!(area_columns(&datar, 100.0, COLUMN_STEP).len(), 1);
//! ```

use silka_paint::{Point, Rect};

/// The width of one rasterisation column, in logical points.
///
/// One point is the resolution at which a reader can see a kink in a line;
/// finer columns buy nothing and cost commands. At 2× DPI this is still half a
/// physical pixel per column.
pub const COLUMN_STEP: f32 = 1.0;

/// The fill between a polyline and a horizontal baseline — an area chart.
///
/// The top edge is walked at exactly [`COLUMN_STEP`] resolution, which is what
/// keeps it flush with the [`silka_paint::Stroke`] drawn along the same polyline:
/// a disagreement of even half a point would show as a hairline of background
/// between the fill and its own line.
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

    #[test]
    fn atap_datar_hanya_satu_kotak() {
        // The whole point of merging: a flat top edge must not cost 600 commands.
        let p = [Point::new(0.0, 20.0), Point::new(600.0, 20.0)];
        let r = area_columns(&p, 100.0, COLUMN_STEP);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].size.width, 600.0);
    }

    #[test]
    fn kolom_menutupi_seluruh_rentang_x_tanpa_celah() {
        // A gap between columns is a striped fill, and it is the kind of bug that
        // only shows up on a screenshot.
        let p = [
            Point::new(0.0, 10.0),
            Point::new(100.0, 90.0),
            Point::new(200.0, 30.0),
        ];
        let r = area_columns(&p, 200.0, COLUMN_STEP);
        assert!(
            (total_width(&r) - 200.0).abs() < 0.01,
            "{}",
            total_width(&r)
        );
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
    fn nilai_nan_kehilangan_ruasnya_bukan_seluruh_seri() {
        let p = [
            Point::new(0.0, 10.0),
            Point::new(50.0, f32::NAN),
            Point::new(100.0, 30.0),
        ];
        let r = area_columns(&p, 100.0, 1.0);
        assert!(r
            .iter()
            .all(|x| x.origin.y.is_finite() && x.size.height.is_finite()));
    }

    #[test]
    fn biaya_tumbuh_dengan_lebar_bukan_dengan_jumlah_titik() {
        // The claim the column walk is built on. A hundred points across 400pt
        // must not cost more than the columns that fit in 400pt.
        let titik: Vec<Point> = (0..100)
            .map(|i| Point::new(i as f32 * 4.0, if i % 2 == 0 { 10.0 } else { 90.0 }))
            .collect();
        let r = area_columns(&titik, 200.0, COLUMN_STEP);
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
