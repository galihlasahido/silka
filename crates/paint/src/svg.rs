//! SVG path → coverage mask, on the CPU.
//!
//! Monochrome icons are the one bitmap a UI toolkit cannot avoid owning: they
//! come as vectors (SF Symbols exports, Lucide, Feather, an in-house set), they
//! are drawn at a handful of sizes, and they must be tintable by a theme token.
//! Rather than growing a general vector renderer — the thing REKOMENDASI §3.2
//! deliberately did *not* buy — this module does the smallest useful job:
//! rasterise one filled path into an alpha mask, once, at load time.
//!
//! The mask then goes straight into [`crate::ImageAtlas::insert_mask`], which
//! stores it as white pixels with the coverage in alpha — so an icon rides in the
//! same atlas, the same texture binding, and the same single draw call as text
//! and boxes, and [`crate::ImageQuad::tint`] colours it from a token.
//!
//! ```
//! use silka_paint::{rasterize_path, FillRule, ImageAtlas};
//!
//! // A 24-unit viewBox icon rasterised for a 16pt slot on a 2x display.
//! let mask = rasterize_path("M4 4 H20 V20 H4 Z", 24.0, 32, FillRule::NonZero)
//!     .expect("path parses");
//! assert_eq!((mask.width(), mask.height()), (32, 32));
//!
//! // Inside the square is opaque, outside is empty, and the edge is anti-aliased
//! // rather than a staircase.
//! assert_eq!(mask.pixel(16, 16), 255);
//! assert_eq!(mask.pixel(1, 1), 0);
//!
//! // Straight into the atlas the backend already uploads.
//! let mut atlas = ImageAtlas::new();
//! let id = atlas.insert_svg_path("M4 4 H20 V20 H4 Z", 24.0, 32).unwrap();
//! let _ = id;
//! ```
//!
//! ## What is supported, and what is refused
//!
//! Commands: `M m L l H h V v C c S s Q q T t Z z`. Elliptical arcs (`A`/`a`) are
//! **refused** — [`rasterize_path`] answers `None` — because an arc converted
//! wrongly is a silently misshapen icon, and refusing gives the caller a chance
//! to convert the path to curves offline instead. There are no strokes here
//! either: a stroked icon is converted to a filled path by its exporter, and if
//! it is not, [`crate::Stroke`] draws it as a real line at draw time.

use crate::geometry::Point;
use crate::image::{ImageAtlas, ImageId};

/// How a self-overlapping path decides what is inside.
///
/// ```
/// use silka_paint::FillRule;
///
/// // A ring drawn as two concentric circles needs even-odd to leave a hole;
/// // one drawn as two opposite-winding contours works under either rule.
/// assert_eq!(FillRule::default(), FillRule::NonZero);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    /// Inside when the winding number is not zero (the SVG default).
    #[default]
    NonZero,
    /// Inside when the crossing count is odd.
    EvenOdd,
}

impl FillRule {
    fn inside(self, winding: i32) -> bool {
        match self {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }
}

/// A rasterised coverage mask: one byte of alpha per pixel.
///
/// ```
/// use silka_paint::{rasterize_path, FillRule};
///
/// let mask = rasterize_path("M0 0 H8 V8 H0 Z", 8.0, 8, FillRule::NonZero).unwrap();
/// assert_eq!(mask.alpha().len(), 64);
/// // Out-of-bounds reads are empty rather than a panic.
/// assert_eq!(mask.pixel(99, 0), 0);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconMask {
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

impl IconMask {
    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The coverage bytes, row by row.
    pub fn alpha(&self) -> &[u8] {
        &self.alpha
    }

    /// One pixel's coverage; `0` outside the mask.
    pub fn pixel(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.alpha[(y * self.width + x) as usize]
    }
}

/// The largest mask this function will produce, per side.
///
/// An icon is a small thing; a request for a 4096² icon is a bug in the caller,
/// and answering `None` is cheaper than allocating 16 MiB to prove it.
const MAX_SIDE: u32 = 512;

/// Vertical samples per pixel row. Four is where the staircase stops being
/// visible on a 16pt icon; more only costs load time.
const SUBSAMPLES: u32 = 4;

/// Rasterise one filled SVG path into a square coverage mask.
///
/// - `d` is the path data (the `d` attribute).
/// - `viewport` is the side of the source `viewBox` in user units (24.0 for a
///   `0 0 24 24` icon set).
/// - `size` is the target side **in pixels** — the caller has already multiplied
///   by the scale factor, because that is the only layer that knows it.
///
/// `None` when the path cannot be parsed, uses an elliptical arc, or `size` is
/// zero or absurd.
pub fn rasterize_path(d: &str, viewport: f32, size: u32, rule: FillRule) -> Option<IconMask> {
    if size == 0 || size > MAX_SIDE || !(viewport.is_finite() && viewport > 0.0) {
        return None;
    }
    let scale = size as f32 / viewport;
    let contours = flatten_path(d, scale)?;
    Some(fill(&contours, size, size, rule))
}

impl ImageAtlas {
    /// Rasterise an SVG path and insert it as a tintable coverage mask.
    ///
    /// The one call an `icon()` widget needs: vector in, atlas handle out, and
    /// the colour still decided by a theme token at draw time.
    pub fn insert_svg_path(&mut self, d: &str, viewport: f32, size: u32) -> Option<ImageId> {
        let mask = rasterize_path(d, viewport, size, FillRule::NonZero)?;
        self.insert_mask(mask.width(), mask.height(), mask.alpha())
    }
}

// ---------------------------------------------------------------------------
// Path parsing + flattening
// ---------------------------------------------------------------------------

/// Flatten a path into polylines, already scaled into pixel space.
///
/// Every curve becomes line segments here, which is what keeps the rasteriser
/// itself a scanline fill with no curve mathematics in it at all.
fn flatten_path(d: &str, scale: f32) -> Option<Vec<Vec<Point>>> {
    let mut scan = Scanner::new(d);
    let mut contours: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    // The pen, in user units.
    let mut pen = Point::ZERO;
    // The start of the current subpath, for `Z`.
    let mut start = Point::ZERO;
    // The reflected control point for `S`/`T`, in user units.
    let mut last_cubic_ctrl: Option<Point> = None;
    let mut last_quad_ctrl: Option<Point> = None;
    let mut command: u8 = 0;

    loop {
        scan.skip_separators();
        if scan.at_end() {
            break;
        }
        if let Some(c) = scan.command() {
            command = c;
        } else if command == 0 {
            // Data before any command at all: malformed.
            return None;
        }

        let relative = command.is_ascii_lowercase();
        let upper = command.to_ascii_uppercase();
        let abs = |p: Point, pen: Point| {
            if relative {
                Point::new(pen.x + p.x, pen.y + p.y)
            } else {
                p
            }
        };

        match upper {
            b'M' => {
                let p = abs(scan.point()?, pen);
                if current.len() > 1 {
                    contours.push(core::mem::take(&mut current));
                } else {
                    current.clear();
                }
                pen = p;
                start = p;
                current.push(scaled(p, scale));
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                // A repeated coordinate pair after `M` means `L` (SVG rule).
                command = if relative { b'l' } else { b'L' };
            }
            b'L' => {
                let p = abs(scan.point()?, pen);
                pen = p;
                current.push(scaled(p, scale));
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'H' => {
                let x = scan.number()?;
                let p = Point::new(if relative { pen.x + x } else { x }, pen.y);
                pen = p;
                current.push(scaled(p, scale));
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'V' => {
                let y = scan.number()?;
                let p = Point::new(pen.x, if relative { pen.y + y } else { y });
                pen = p;
                current.push(scaled(p, scale));
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'C' | b'S' => {
                let (c1, c2, to) = if upper == b'C' {
                    let c1 = abs(scan.point()?, pen);
                    let c2 = abs(scan.point()?, pen);
                    let to = abs(scan.point()?, pen);
                    (c1, c2, to)
                } else {
                    // `S`: the first control point mirrors the previous one.
                    let c1 = match last_cubic_ctrl {
                        Some(prev) => Point::new(2.0 * pen.x - prev.x, 2.0 * pen.y - prev.y),
                        None => pen,
                    };
                    let c2 = abs(scan.point()?, pen);
                    let to = abs(scan.point()?, pen);
                    (c1, c2, to)
                };
                push_cubic(&mut current, pen, c1, c2, to, scale);
                pen = to;
                last_cubic_ctrl = Some(c2);
                last_quad_ctrl = None;
            }
            b'Q' | b'T' => {
                let (ctrl, to) = if upper == b'Q' {
                    let c = abs(scan.point()?, pen);
                    let to = abs(scan.point()?, pen);
                    (c, to)
                } else {
                    let c = match last_quad_ctrl {
                        Some(prev) => Point::new(2.0 * pen.x - prev.x, 2.0 * pen.y - prev.y),
                        None => pen,
                    };
                    let to = abs(scan.point()?, pen);
                    (c, to)
                };
                push_quadratic(&mut current, pen, ctrl, to, scale);
                pen = to;
                last_quad_ctrl = Some(ctrl);
                last_cubic_ctrl = None;
            }
            b'Z' => {
                if current.len() > 1 {
                    current.push(scaled(start, scale));
                    contours.push(core::mem::take(&mut current));
                } else {
                    current.clear();
                }
                pen = start;
                current.push(scaled(start, scale));
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            // An arc converted wrongly is a silently misshapen icon; refusing
            // gives the caller the chance to convert it offline instead.
            b'A' => return None,
            _ => return None,
        }
    }

    if current.len() > 1 {
        contours.push(current);
    }
    if contours.is_empty() {
        return None;
    }
    Some(contours)
}

fn scaled(p: Point, scale: f32) -> Point {
    Point::new(p.x * scale, p.y * scale)
}

/// How many line segments a curve becomes: one per pixel of control-polygon
/// length, bounded so a pathological path cannot allocate without limit.
fn segments_for(length_px: f32) -> u32 {
    if !length_px.is_finite() {
        return 1;
    }
    (length_px.ceil() as u32).clamp(1, 64)
}

fn push_cubic(out: &mut Vec<Point>, from: Point, c1: Point, c2: Point, to: Point, scale: f32) {
    let a = scaled(from, scale);
    let b = scaled(c1, scale);
    let c = scaled(c2, scale);
    let d = scaled(to, scale);
    let n = segments_for(dist(a, b) + dist(b, c) + dist(c, d));
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let x = u * u * u * a.x + 3.0 * u * u * t * b.x + 3.0 * u * t * t * c.x + t * t * t * d.x;
        let y = u * u * u * a.y + 3.0 * u * u * t * b.y + 3.0 * u * t * t * c.y + t * t * t * d.y;
        out.push(Point::new(x, y));
    }
}

fn push_quadratic(out: &mut Vec<Point>, from: Point, ctrl: Point, to: Point, scale: f32) {
    let a = scaled(from, scale);
    let b = scaled(ctrl, scale);
    let c = scaled(to, scale);
    let n = segments_for(dist(a, b) + dist(b, c));
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let x = u * u * a.x + 2.0 * u * t * b.x + t * t * c.x;
        let y = u * u * a.y + 2.0 * u * t * b.y + t * t * c.y;
        out.push(Point::new(x, y));
    }
}

fn dist(a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    (dx * dx + dy * dy).sqrt()
}

// ---------------------------------------------------------------------------
// Scanline fill
// ---------------------------------------------------------------------------

/// One edge of the flattened path, in pixel space.
#[derive(Debug, Clone, Copy)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    /// +1 when the edge runs downwards, -1 upwards — the winding contribution.
    direction: i32,
}

/// Scanline fill with vertical supersampling and analytic horizontal coverage.
///
/// Anti-aliasing is not an afterthought here: horizontal coverage is computed as
/// the exact overlap of the span with each pixel, and only the vertical axis is
/// sampled. That is why four sub-rows are enough for an icon edge to look smooth
/// at 16pt.
fn fill(contours: &[Vec<Point>], width: u32, height: u32, rule: FillRule) -> IconMask {
    let mut edges: Vec<Edge> = Vec::new();
    for contour in contours {
        if contour.len() < 2 {
            continue;
        }
        let n = contour.len();
        for i in 0..n {
            let a = contour[i];
            // Filling always treats a contour as closed, whether or not the path
            // said `Z` — an open filled path is filled as if it were closed
            // (the SVG rule).
            let b = contour[(i + 1) % n];
            if !a.x.is_finite() || !a.y.is_finite() || !b.x.is_finite() || !b.y.is_finite() {
                continue;
            }
            if (a.y - b.y).abs() < f32::EPSILON {
                // A horizontal edge crosses no sample row, so it contributes
                // nothing but would divide by zero.
                continue;
            }
            edges.push(Edge {
                x0: a.x,
                y0: a.y,
                x1: b.x,
                y1: b.y,
                direction: if b.y > a.y { 1 } else { -1 },
            });
        }
    }

    let mut coverage = vec![0.0f32; (width * height) as usize];
    let weight = 1.0 / SUBSAMPLES as f32;
    let mut crossings: Vec<(f32, i32)> = Vec::with_capacity(16);

    for py in 0..height {
        let row = (py * width) as usize;
        for s in 0..SUBSAMPLES {
            let y = py as f32 + (s as f32 + 0.5) / SUBSAMPLES as f32;
            crossings.clear();
            for e in &edges {
                let (top, bottom) = (e.y0.min(e.y1), e.y0.max(e.y1));
                // Half-open in y so a vertex shared by two edges is counted
                // exactly once — the classic double-count bug.
                if y < top || y >= bottom {
                    continue;
                }
                let t = (y - e.y0) / (e.y1 - e.y0);
                crossings.push((e.x0 + t * (e.x1 - e.x0), e.direction));
            }
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding = 0;
            let mut span_start = 0.0f32;
            for (x, dir) in &crossings {
                let was_inside = rule.inside(winding);
                winding += dir;
                let now_inside = rule.inside(winding);
                if !was_inside && now_inside {
                    span_start = *x;
                } else if was_inside && !now_inside {
                    add_span(
                        &mut coverage[row..row + width as usize],
                        span_start,
                        *x,
                        weight,
                    );
                }
            }
        }
    }

    IconMask {
        width,
        height,
        alpha: coverage
            .into_iter()
            .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect(),
    }
}

/// Add one horizontal span's coverage to a pixel row, partial pixels included.
fn add_span(row: &mut [f32], x0: f32, x1: f32, weight: f32) {
    let width = row.len() as f32;
    // `max`/`min` also launder a NaN into a bound, so a malformed path cannot
    // poison a whole row.
    let x0 = x0.max(0.0);
    let x1 = x1.min(width);
    if x1 <= x0 {
        return;
    }
    let first = x0.floor() as usize;
    let last = (x1.ceil() as usize).min(row.len());
    for (i, pixel) in row.iter_mut().enumerate().take(last).skip(first) {
        let left = (i as f32).max(x0);
        let right = ((i + 1) as f32).min(x1);
        if right > left {
            *pixel += (right - left) * weight;
        }
    }
}

// ---------------------------------------------------------------------------
// Number scanner
// ---------------------------------------------------------------------------

/// A tolerant scanner for SVG path data.
///
/// Path data in the wild is compact rather than tidy: `M0 0l4-4.5.5.5z` is legal
/// and common, so separators are optional and a sign or a second decimal point
/// starts a new number.
struct Scanner<'a> {
    bytes: &'a [u8],
    i: usize,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            i: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.i >= self.bytes.len()
    }

    fn skip_separators(&mut self) {
        while let Some(b) = self.bytes.get(self.i) {
            if b.is_ascii_whitespace() || *b == b',' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    /// The next command letter, if the next byte is one.
    fn command(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.i)?;
        if b.is_ascii_alphabetic() {
            self.i += 1;
            Some(b)
        } else {
            None
        }
    }

    /// The next number, or `None` when what follows is not one.
    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.i;
        if matches!(self.bytes.get(self.i), Some(b'+') | Some(b'-')) {
            self.i += 1;
        }
        let mut digits = false;
        while let Some(b) = self.bytes.get(self.i) {
            if b.is_ascii_digit() {
                digits = true;
                self.i += 1;
            } else {
                break;
            }
        }
        if self.bytes.get(self.i) == Some(&b'.') {
            self.i += 1;
            while let Some(b) = self.bytes.get(self.i) {
                if b.is_ascii_digit() {
                    digits = true;
                    self.i += 1;
                } else {
                    break;
                }
            }
        }
        if !digits {
            self.i = start;
            return None;
        }
        // An exponent only counts when it is followed by digits; otherwise the
        // `e` belongs to whatever comes next.
        if matches!(self.bytes.get(self.i), Some(b'e') | Some(b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.bytes.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            let mut exp_digits = false;
            while let Some(b) = self.bytes.get(self.i) {
                if b.is_ascii_digit() {
                    exp_digits = true;
                    self.i += 1;
                } else {
                    break;
                }
            }
            if !exp_digits {
                self.i = save;
            }
        }
        let text = core::str::from_utf8(&self.bytes[start..self.i]).ok()?;
        text.parse::<f32>().ok().filter(|v| v.is_finite())
    }

    /// The next coordinate pair.
    fn point(&mut self) -> Option<Point> {
        let x = self.number()?;
        let y = self.number()?;
        Some(Point::new(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: u32) -> IconMask {
        // A 24-unit viewBox with an 8..16 square in the middle.
        rasterize_path("M8 8 H16 V16 H8 Z", 24.0, size, FillRule::NonZero).expect("terparse")
    }

    #[test]
    fn kotak_terisi_di_dalam_kosong_di_luar() {
        let m = square(24);
        assert_eq!(m.pixel(12, 12), 255, "tengah harus penuh");
        assert_eq!(m.pixel(2, 2), 0, "luar harus kosong");
        assert_eq!(m.pixel(21, 21), 0);
        assert_eq!(m.alpha().len(), 24 * 24);
    }

    #[test]
    fn tepi_dianti_alias_bukan_tangga() {
        // A diagonal is the case that separates a real rasteriser from a
        // nearest-neighbour one: some pixels must be partially covered.
        let m = rasterize_path("M0 0 L24 24 L0 24 Z", 24.0, 24, FillRule::NonZero).unwrap();
        let sebagian = m.alpha().iter().filter(|a| **a > 0 && **a < 255).count();
        assert!(sebagian > 8, "hanya {sebagian} piksel separuh");
    }

    #[test]
    fn ukuran_target_menentukan_resolusi_bukan_viewbox() {
        // The same icon at 1x and 2x: same shape, twice the pixels.
        let kecil = square(16);
        let besar = square(32);
        assert_eq!((kecil.width(), kecil.height()), (16, 16));
        assert_eq!((besar.width(), besar.height()), (32, 32));
        // The middle is inside in both.
        assert_eq!(kecil.pixel(8, 8), 255);
        assert_eq!(besar.pixel(16, 16), 255);
    }

    #[test]
    fn perintah_relatif_sama_dengan_absolut() {
        let absolut = rasterize_path("M4 4 H20 V20 H4 Z", 24.0, 24, FillRule::NonZero).unwrap();
        let relatif = rasterize_path("m4 4 h16 v16 h-16 z", 24.0, 24, FillRule::NonZero).unwrap();
        assert_eq!(absolut.alpha(), relatif.alpha());
    }

    #[test]
    fn pasangan_koordinat_berulang_setelah_m_berarti_l() {
        // The SVG rule that trips naive parsers: after `M` a second pair is a
        // line, not another move.
        let implisit = rasterize_path("M2 2 22 2 22 22 2 22 Z", 24.0, 24, FillRule::NonZero);
        let eksplisit = rasterize_path("M2 2 L22 2 L22 22 L2 22 Z", 24.0, 24, FillRule::NonZero);
        assert_eq!(
            implisit.map(|m| m.alpha().to_vec()),
            eksplisit.map(|m| m.alpha().to_vec())
        );
    }

    #[test]
    fn kurva_kubik_menghasilkan_bentuk_membulat() {
        // A quarter disc: the corner opposite the curve must be empty while the
        // interior is full — proof the curve was flattened, not skipped.
        let m = rasterize_path(
            "M0 24 C0 10 10 0 24 0 L24 24 Z",
            24.0,
            48,
            FillRule::NonZero,
        )
        .unwrap();
        assert_eq!(m.pixel(40, 40), 255, "sisi lurus harus terisi");
        assert_eq!(m.pixel(2, 2), 0, "sudut di luar kurva harus kosong");
    }

    #[test]
    fn kurva_kuadratik_dan_halus_diterima() {
        assert!(rasterize_path("M0 24 Q0 0 24 0 Z", 24.0, 24, FillRule::NonZero).is_some());
        assert!(rasterize_path("M0 24 Q0 12 12 12 T24 0 Z", 24.0, 24, FillRule::NonZero).is_some());
        assert!(rasterize_path(
            "M0 0 C4 0 8 4 8 8 S16 16 24 16 Z",
            24.0,
            24,
            FillRule::NonZero
        )
        .is_some());
    }

    #[test]
    fn even_odd_meninggalkan_lubang() {
        // Two nested squares wound the same way: only even-odd punches a hole,
        // which is precisely why the rule is a parameter.
        let d = "M0 0 H24 V24 H0 Z M6 6 H18 V18 H6 Z";
        let nonzero = rasterize_path(d, 24.0, 24, FillRule::NonZero).unwrap();
        let evenodd = rasterize_path(d, 24.0, 24, FillRule::EvenOdd).unwrap();
        assert_eq!(nonzero.pixel(12, 12), 255);
        assert_eq!(evenodd.pixel(12, 12), 0, "harus berlubang");
        // The outer ring is filled either way.
        assert_eq!(evenodd.pixel(2, 12), 255);
    }

    #[test]
    fn busur_ditolak_bukan_digambar_salah() {
        assert!(rasterize_path("M0 0 A5 5 0 0 1 10 10 Z", 24.0, 24, FillRule::NonZero).is_none());
    }

    #[test]
    fn masukan_ngawur_ditolak() {
        assert!(rasterize_path("", 24.0, 24, FillRule::NonZero).is_none());
        assert!(rasterize_path("4 4 8 8", 24.0, 24, FillRule::NonZero).is_none());
        assert!(rasterize_path("M0 0 X9", 24.0, 24, FillRule::NonZero).is_none());
        assert!(rasterize_path("M8 8 H16 V16 H8 Z", 0.0, 24, FillRule::NonZero).is_none());
        assert!(rasterize_path("M8 8 H16 V16 H8 Z", 24.0, 0, FillRule::NonZero).is_none());
        assert!(rasterize_path("M8 8 H16 V16 H8 Z", 24.0, 9999, FillRule::NonZero).is_none());
        // A move with no coordinates is malformed, not an empty icon.
        assert!(rasterize_path("M", 24.0, 24, FillRule::NonZero).is_none());
    }

    #[test]
    fn angka_padat_terbaca() {
        let mut s = Scanner::new("4-4.5.5.5e1,+2 1e");
        assert_eq!(s.number(), Some(4.0));
        assert_eq!(s.number(), Some(-4.5));
        assert_eq!(s.number(), Some(0.5));
        assert_eq!(s.number(), Some(5.0));
        assert_eq!(s.number(), Some(2.0));
        // `1e` with no exponent digits is the number 1 followed by a letter.
        assert_eq!(s.number(), Some(1.0));
        assert_eq!(s.command(), Some(b'e'));
        assert!(s.at_end());
    }

    #[test]
    fn mask_di_luar_batas_nol() {
        let m = square(8);
        assert_eq!(m.pixel(8, 0), 0);
        assert_eq!(m.pixel(0, 8), 0);
    }

    #[test]
    fn span_menambahkan_cakupan_sebagian() {
        let mut row = vec![0.0f32; 4];
        add_span(&mut row, 0.5, 2.25, 1.0);
        assert!((row[0] - 0.5).abs() < 1e-5);
        assert!((row[1] - 1.0).abs() < 1e-5);
        assert!((row[2] - 0.25).abs() < 1e-5);
        assert_eq!(row[3], 0.0);
        // Outside the row is clipped rather than panicking.
        add_span(&mut row, -10.0, 100.0, 1.0);
        assert!(row.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn atlas_menerima_jalur_svg_langsung() {
        let mut atlas = ImageAtlas::new();
        let id = atlas
            .insert_svg_path("M4 4 H20 V20 H4 Z", 24.0, 32)
            .expect("masuk atlas");
        use crate::image::ImageSource;
        let letak = atlas.placement(id).expect("punya letak");
        assert_eq!((letak.width, letak.height), (32, 32));
        assert!(atlas
            .insert_svg_path("M0 0 A1 1 0 0 1 2 2", 24.0, 32)
            .is_none());
    }
}
