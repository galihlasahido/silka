//! **Dragging files out of the window** — the one P0 item in
//! `INTEGRASI-NATIVE.md` (§4) with no crate behind it, and until this example
//! existed, the one nothing in this repository had ever actually used.
//!
//! `silka_platform::drag` was written in Fase 5 with its vocabulary tested and
//! its macOS backend implemented, but a drag source is not proved by unit
//! tests: it is proved by a file landing in a Finder window. This module is the
//! part of that which is not AppKit, and every piece of it is a pure function
//! with a test, because the alternative is debugging a gesture by hand.
//!
//! | Question | Answered by |
//! |---|---|
//! | has the pointer moved far enough to be a drag, not a click? | [`started`] |
//! | which row is under the pointer, given the scroll offset? | [`RowHits::row_at`] |
//! | what exactly is on offer, and in what order? | [`source_for`] |
//! | what does the pointer carry while it drags? | [`preview_bitmap`] |
//! | what does the application do once the drop lands? | [`after_drop`] |
//!
//! ## Why a move does not delete anything here
//!
//! [`DragEffect::source_must_remove`] is true for a move, and for an
//! application dragging its *own* data — a row out of a table, a note between
//! two windows — that is exactly right: nobody else can remove it.
//!
//! A `public.file-url` drag is the exception, and getting it wrong destroys a
//! user's file. The receiver of a file URL is a file manager, and a file
//! manager performs the move **itself**: by the time the drop reports
//! `NSDragOperationMove` the file is already at its new home. Deleting "our
//! copy" afterwards would be deleting the file the user just moved. So
//! [`after_drop`] answers a move with a rescan and never with a delete —
//! stated here rather than discovered later.

use std::path::{Path, PathBuf};

use silka_paint::{Color, Point, Rect};
use silka_platform::drag::{drag, DragEffect, DragEffects, DragPreview, DragSource};
use silka_platform::image::RgbaImage;

/// How far the pointer must travel, with the button down, before a press
/// becomes a drag.
///
/// Four points. Small enough that a deliberate drag starts immediately, large
/// enough that a click with a shaky hand is still a click — which matters more
/// than it sounds, because a click that turns into a drag steals the selection
/// gesture the user meant.
pub const DRAG_THRESHOLD: f32 = 4.0;

/// Whether the pointer has moved far enough from where it was pressed.
///
/// Compared on the square of the distance, so nothing takes a square root on
/// every mouse-move event.
///
/// ```text
/// started((10,10), (12,10), 4.0) == false   // still a click
/// started((10,10), (16,10), 4.0) == true    // a drag
/// ```
pub fn started(press: Point, now: Point, threshold: f32) -> bool {
    let dx = now.x - press.x;
    let dy = now.y - press.y;
    let d2 = dx * dx + dy * dy;
    // A non-finite position is what a disconnected pointer reports; treating it
    // as "the drag started" would fling a file at nothing.
    d2.is_finite() && d2 > threshold * threshold
}

// ---------------------------------------------------------------------------
// Which row is under the pointer
// ---------------------------------------------------------------------------

/// Enough geometry to say which listing row a window point falls on.
///
/// The listing is virtualized, so there is no widget to ask: the rows under the
/// pointer may not have been built yet. What there *is* is the arithmetic the
/// list itself uses — origin, row height, scroll offset — and repeating it here
/// is cheaper and more honest than reaching into a render node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowHits {
    /// The listing's viewport, in window points.
    pub viewport: Rect,
    /// The height of one row.
    pub row_extent: f32,
    /// How far the list is scrolled.
    pub offset: f32,
    /// How many rows there are in total.
    pub count: usize,
}

impl RowHits {
    /// A listing that is not on screen at all — no row is ever under the
    /// pointer.
    pub const NONE: RowHits = RowHits {
        viewport: Rect::new(0.0, 0.0, 0.0, 0.0),
        row_extent: 1.0,
        offset: 0.0,
        count: 0,
    };

    /// The row index at a window point, if the point is over a row.
    pub fn row_at(&self, point: Point) -> Option<usize> {
        if self.count == 0 || self.row_extent <= 0.0 || !self.viewport.contains(point) {
            return None;
        }
        let local = point.y - self.viewport.min_y() + self.offset;
        if local < 0.0 || !local.is_finite() {
            return None;
        }
        let index = (local / self.row_extent) as usize;
        (index < self.count).then_some(index)
    }
}

// ---------------------------------------------------------------------------
// What is on offer
// ---------------------------------------------------------------------------

/// The text a drag carries alongside the files.
///
/// One file offers its name; several offer their names one per line, which is
/// what a text editor receiving the drop expects and what a terminal can
/// actually use.
pub fn drag_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| p.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The caption under the pointer: `report.pdf`, or `3 items`.
pub fn drag_caption(paths: &[PathBuf]) -> String {
    match paths.len() {
        0 => String::new(),
        1 => drag_text(paths),
        n => format!("{n} items"),
    }
}

/// Describe the drag: the files first, their names as text second.
///
/// Order **is** preference order (`silka_platform::drag`): a Finder window
/// takes the file URLs, a text editor takes the names. Getting them the wrong
/// way round means dragging a file into a folder deposits a text file
/// containing its name.
///
/// The permitted effects are copy, move and link — everything a file manager
/// offers — but see the module documentation for why a move does not delete
/// anything here.
pub fn source_for(paths: &[PathBuf], preview: DragPreview) -> DragSource {
    drag()
        .files(paths.to_vec())
        .text(drag_text(paths))
        .allow(DragEffects::ALL)
        .preview(preview)
}

/// What the application has to do once a drop has landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followup {
    /// Nothing at all: the files are exactly where they were.
    None,
    /// The receiver relocated the files; the folder on screen is out of date.
    Rescan,
}

/// The follow-up a finished drag calls for.
///
/// Never a delete. See the module documentation: for a file URL the receiver
/// has already performed the move, and "the source removes its copy" would be
/// removing the file the user just moved.
pub fn after_drop(effect: Option<DragEffect>) -> Followup {
    match effect {
        Some(DragEffect::Move) => Followup::Rescan,
        // A copy or a link leaves the source folder untouched, and `None` is a
        // drag that landed nowhere.
        _ => Followup::None,
    }
}

// ---------------------------------------------------------------------------
// The image that follows the pointer
// ---------------------------------------------------------------------------

/// The card that follows the pointer during a drag.
///
/// Deliberately not a screenshot of the row. Reading pixels back out of the
/// frame the GPU is drawing means a readback and a stall, in the middle of a
/// gesture whose whole job is to feel immediate; and the framework has no API
/// for rasterising a view off-screen on demand. So the preview is drawn here:
/// a rounded, translucent card in the theme's own colours, with a stripe in the
/// colour of the file's kind down its leading edge.
///
/// `scale` is pixels per point — pass the window's scale factor, or the preview
/// is half the size it should be on a Retina display.
///
/// **Acknowledged debt:** the card carries no text. Rasterising the file's name
/// into a standalone bitmap means reaching into the glyph atlas from outside a
/// frame, which is not something `silka-text` exposes today. The caption is
/// still computed ([`drag_caption`]) and shown in the window while the drag is
/// armed, so the user is never left guessing what they picked up.
pub fn preview_bitmap(
    width: f32,
    height: f32,
    scale: f32,
    fill: Color,
    border: Color,
    stripe: Color,
) -> Option<RgbaImage> {
    let scale = if scale.is_finite() && scale >= 1.0 {
        scale.round().min(4.0)
    } else {
        1.0
    };
    // Clamped rather than trusted: a window that reports a nonsense size must
    // not turn into a gigabyte of drag preview.
    let w_pt = width.clamp(24.0, 480.0);
    let h_pt = height.clamp(16.0, 160.0);
    let pw = (w_pt * scale).round() as u32;
    let ph = (h_pt * scale).round() as u32;

    let radius = 6.0 * scale;
    let border_w = 1.0 * scale;
    let stripe_w = 4.0 * scale;
    let mut pixels = vec![0u8; (pw as usize) * (ph as usize) * 4];

    for y in 0..ph {
        for x in 0..pw {
            // Sampled at the pixel centre; sampling at the corner puts the
            // whole card half a pixel up and to the left.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = rounded_rect_distance(px, py, pw as f32, ph as f32, radius);
            // One pixel of feathering, which is what stops the corners looking
            // like a staircase at 1×.
            let coverage = (0.5 - d).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let inside_border = d < -border_w;
            let base = if !inside_border {
                border
            } else if px < stripe_w + border_w {
                stripe
            } else {
                fill
            };
            let at = ((y as usize) * (pw as usize) + x as usize) * 4;
            let alpha = base.a * coverage;
            // Premultiplied: AppKit and every compositor underneath it expect
            // the colour to already carry its own alpha, and a straight-alpha
            // bitmap shows up with bright fringes on its edges.
            pixels[at] = to_byte(base.r * alpha);
            pixels[at + 1] = to_byte(base.g * alpha);
            pixels[at + 2] = to_byte(base.b * alpha);
            pixels[at + 3] = to_byte(alpha);
        }
    }

    RgbaImage::new(pw, ph, pixels).ok()
}

/// Signed distance from a point to a rounded rectangle covering the whole
/// bitmap — negative inside, positive outside.
fn rounded_rect_distance(x: f32, y: f32, w: f32, h: f32, radius: f32) -> f32 {
    let radius = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    // Distance to the inner rectangle whose corners the radius rounds off.
    let dx = (radius - x).max(x - (w - radius)).max(0.0);
    let dy = (radius - y).max(y - (h - radius)).max(0.0);
    let corner = (dx * dx + dy * dy).sqrt() - radius;
    if dx > 0.0 || dy > 0.0 {
        corner
    } else {
        // Inside the straight part: the distance to the nearest edge, inset by
        // the radius so the sign is continuous across the corner boundary.
        let ex = (x - radius).min((w - radius) - x);
        let ey = (y - radius).min((h - radius) - y);
        -(ex.min(ey) + radius)
    }
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A preview centred on the point that was grabbed.
///
/// The hotspot is where the pointer sits **inside** the card. Putting it where
/// the press landed relative to the row is what makes the card feel picked up
/// rather than teleported into the hand.
pub fn preview_for(image: RgbaImage, scale: f32, press: Point, row_origin: Point) -> DragPreview {
    DragPreview::new(
        image,
        scale,
        Point::new(press.x - row_origin.x, press.y - row_origin.y),
    )
}

/// Whether a path is worth dragging at all.
///
/// A placeholder row ("Loading…") has no path, and a drag of nothing is a drag
/// the user cannot tell failed.
pub fn is_draggable(path: &Path) -> bool {
    path.file_name().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_platform::drag::DragItem;

    fn colors() -> (Color, Color, Color) {
        (
            Color::srgba(0.2, 0.2, 0.25, 0.92),
            Color::srgba(1.0, 1.0, 1.0, 0.6),
            Color::srgba(0.0, 0.5, 1.0, 1.0),
        )
    }

    #[test]
    fn ambang_membedakan_klik_dari_seret() {
        let press = Point::new(10.0, 10.0);
        assert!(!started(press, Point::new(12.0, 10.0), DRAG_THRESHOLD));
        assert!(!started(press, press, DRAG_THRESHOLD));
        assert!(started(press, Point::new(16.0, 10.0), DRAG_THRESHOLD));
        // Diagonal counts: 3-4-5 is five points away.
        assert!(started(press, Point::new(13.0, 14.0), DRAG_THRESHOLD));
    }

    #[test]
    fn posisi_tidak_masuk_akal_tidak_memulai_seretan() {
        let press = Point::new(10.0, 10.0);
        assert!(!started(press, Point::new(f32::NAN, 10.0), DRAG_THRESHOLD));
        assert!(!started(
            press,
            Point::new(f32::INFINITY, 10.0),
            DRAG_THRESHOLD
        ));
    }

    #[test]
    fn baris_dihitung_dari_gulungan_bukan_dari_layar() {
        let hits = RowHits {
            viewport: Rect::new(200.0, 100.0, 600.0, 400.0),
            row_extent: 28.0,
            offset: 0.0,
            count: 100,
        };
        assert_eq!(hits.row_at(Point::new(300.0, 100.0)), Some(0));
        assert_eq!(hits.row_at(Point::new(300.0, 127.0)), Some(0));
        assert_eq!(hits.row_at(Point::new(300.0, 128.0)), Some(1));

        // Scrolled down by ten rows: the same pixel is now a different row.
        let scrolled = RowHits {
            offset: 280.0,
            ..hits
        };
        assert_eq!(scrolled.row_at(Point::new(300.0, 100.0)), Some(10));
    }

    #[test]
    fn di_luar_viewport_bukan_baris_apa_pun() {
        let hits = RowHits {
            viewport: Rect::new(200.0, 100.0, 600.0, 400.0),
            row_extent: 28.0,
            offset: 0.0,
            count: 100,
        };
        // In the sidebar, above the list, below it, and past the last row.
        assert_eq!(hits.row_at(Point::new(50.0, 200.0)), None);
        assert_eq!(hits.row_at(Point::new(300.0, 50.0)), None);
        assert_eq!(hits.row_at(Point::new(300.0, 600.0)), None);
        assert_eq!(RowHits::NONE.row_at(Point::new(0.0, 0.0)), None);

        let short = RowHits { count: 2, ..hits };
        assert_eq!(short.row_at(Point::new(300.0, 400.0)), None);
    }

    #[test]
    fn berkas_ditawarkan_sebelum_teks() {
        // The wrong way round means dropping a file into a folder deposits a
        // text file containing its name.
        let paths = vec![PathBuf::from("/tmp/report.pdf")];
        let source = source_for(&paths, preview());
        assert_eq!(source.items().len(), 2);
        assert!(matches!(source.items()[0], DragItem::Files(_)));
        assert!(matches!(source.items()[1], DragItem::Text(_)));
        assert!(source.check().is_ok());
    }

    #[test]
    fn seretan_banyak_berkas_membawa_semuanya_dalam_satu_item() {
        let paths = vec![
            PathBuf::from("/tmp/a.txt"),
            PathBuf::from("/tmp/b.txt"),
            PathBuf::from("/tmp/c.txt"),
        ];
        let source = source_for(&paths, preview());
        match &source.items()[0] {
            DragItem::Files(files) => assert_eq!(files.len(), 3),
            other => panic!("expected a file item, got {other:?}"),
        }
        assert_eq!(drag_text(&paths), "a.txt\nb.txt\nc.txt");
        assert_eq!(drag_caption(&paths), "3 items");
        assert_eq!(
            drag_caption(&paths[..1]),
            "a.txt",
            "one file is named, not counted"
        );
    }

    #[test]
    fn semua_efek_diizinkan_tapi_pindah_tidak_pernah_menghapus() {
        let source = source_for(&[PathBuf::from("/tmp/a")], preview());
        assert!(source.allowed().contains(DragEffects::MOVE));
        // …and yet the follow-up to a move is a rescan, never a delete. This
        // is the assertion that keeps somebody from "fixing" it later.
        assert_eq!(after_drop(Some(DragEffect::Move)), Followup::Rescan);
        assert_eq!(after_drop(Some(DragEffect::Copy)), Followup::None);
        assert_eq!(after_drop(Some(DragEffect::Link)), Followup::None);
        assert_eq!(after_drop(None), Followup::None);
    }

    fn preview() -> DragPreview {
        let (fill, border, stripe) = colors();
        DragPreview::centered(
            preview_bitmap(200.0, 28.0, 1.0, fill, border, stripe).expect("a preview bitmap"),
            1.0,
        )
    }

    #[test]
    fn pratinjau_punya_sudut_membulat_dan_tengah_yang_pekat() {
        let (fill, border, stripe) = colors();
        let img = preview_bitmap(200.0, 28.0, 1.0, fill, border, stripe).expect("bitmap");
        assert_eq!(img.width(), 200);
        assert_eq!(img.height(), 28);
        // The very corner is outside the rounded rectangle: fully transparent.
        assert_eq!(img.pixel(0, 0).expect("corner")[3], 0);
        // The middle is the fill, at very nearly its own alpha.
        let middle = img.pixel(120, 14).expect("middle");
        assert!(
            middle[3] > 200,
            "the card is opaque in the middle: {middle:?}"
        );
        // The leading edge carries the kind stripe, which is a different colour
        // from the fill — this is what tells a user *what* they picked up.
        let leading = img.pixel(3, 14).expect("stripe");
        assert_ne!(leading[..3], middle[..3]);
    }

    #[test]
    fn pratinjau_retina_dua_kali_lipat_pikselnya_bukan_ukurannya() {
        let (fill, border, stripe) = colors();
        let img = preview_bitmap(200.0, 28.0, 2.0, fill, border, stripe).expect("bitmap");
        assert_eq!((img.width(), img.height()), (400, 56));
        let preview = DragPreview::new(img, 2.0, Point::new(10.0, 14.0));
        // Still 200 x 28 *points* — the size AppKit will draw it at.
        assert_eq!(preview.size().width, 200.0);
        assert_eq!(preview.size().height, 28.0);
    }

    #[test]
    fn ukuran_gila_dijepit_bukan_dialokasikan() {
        let (fill, border, stripe) = colors();
        let huge = preview_bitmap(100_000.0, 100_000.0, 4.0, fill, border, stripe).expect("bitmap");
        assert_eq!(huge.width(), 480 * 4);
        assert_eq!(huge.height(), 160 * 4);
        let tiny = preview_bitmap(0.0, 0.0, 1.0, fill, border, stripe).expect("bitmap");
        assert_eq!((tiny.width(), tiny.height()), (24, 16));
        // A nonsense scale factor is one, not a panic.
        let odd = preview_bitmap(40.0, 20.0, f32::NAN, fill, border, stripe).expect("bitmap");
        assert_eq!((odd.width(), odd.height()), (40, 20));
    }

    #[test]
    fn hotspot_mengikuti_tempat_baris_digenggam() {
        let (fill, border, stripe) = colors();
        let img = preview_bitmap(200.0, 28.0, 1.0, fill, border, stripe).expect("bitmap");
        let preview = preview_for(img, 1.0, Point::new(340.0, 214.0), Point::new(300.0, 200.0));
        assert_eq!(preview.hotspot(), Point::new(40.0, 14.0));
    }

    #[test]
    fn jarak_bertanda_negatif_di_dalam_positif_di_luar() {
        // The corner of the bounding box is outside a rounded rectangle…
        assert!(rounded_rect_distance(0.5, 0.5, 100.0, 40.0, 8.0) > 0.0);
        // …the middle is well inside…
        assert!(rounded_rect_distance(50.0, 20.0, 100.0, 40.0, 8.0) < -8.0);
        // …and a point just inside the straight edge is only just inside.
        let edge = rounded_rect_distance(50.0, 0.5, 100.0, 40.0, 8.0);
        assert!(edge < 0.0 && edge > -1.0, "edge distance was {edge}");
    }

    #[test]
    fn baris_tanpa_lintasan_tidak_bisa_diseret() {
        assert!(is_draggable(Path::new("/tmp/a.txt")));
        assert!(!is_draggable(Path::new("")));
        // Neither is the filesystem root: there is no such file to hand over.
        assert!(!is_draggable(Path::new("/")));
    }
}
