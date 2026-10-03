//! The arithmetic behind a drag image, kept free of any OS type so it can be
//! tested everywhere.
//!
//! The Windows shell wants a 32-bit top-down DIB whose colour channels are
//! already multiplied by alpha, in B, G, R, A byte order, plus the pixel
//! offset of the cursor inside it. [`RgbaImage`](crate::image::RgbaImage) is
//! the opposite on every count: straight alpha, R, G, B, A order, and a
//! hotspot in logical points. Getting any of the three wrong is quiet — the
//! drag still works, the thumbnail just has a dark fringe, swapped red and blue
//! or sits offset from the pointer.

use silka_paint::Point;

/// The largest side, in pixels, a drag image is handed to the shell at.
///
/// The shell composites the image under the cursor on every mouse move; an
/// image far bigger than the screen is not a better thumbnail, only a slower
/// one, and a corrupt or hostile size would otherwise decide how much memory
/// this module allocates.
pub(crate) const MAX_SIDE: u32 = 1024;

/// Straight-alpha RGBA into premultiplied BGRA, the layout of a 32-bit DIB
/// with alpha.
///
/// Each colour channel is scaled by `alpha / 255` with rounding, so a fully
/// opaque pixel is unchanged and a fully transparent one becomes all zero.
/// Returns an empty vector when `rgba` is not a whole number of pixels.
pub(crate) fn premultiplied_bgra(rgba: &[u8]) -> Vec<u8> {
    if rgba.len() % 4 != 0 {
        return Vec::new();
    }
    let scale = |channel: u8, alpha: u8| -> u8 {
        ((u32::from(channel) * u32::from(alpha) + 127) / 255) as u8
    };
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        out.extend_from_slice(&[scale(b, a), scale(g, a), scale(r, a), a]);
    }
    out
}

/// The size to hand the shell and the hotspot inside it, both in pixels.
///
/// `scale` is image pixels per logical point, so `hotspot` (logical points)
/// becomes `hotspot * scale` pixels. The result is clamped into the image: a
/// cursor offset outside the bitmap places the thumbnail somewhere unrelated
/// to the pointer.
///
/// Returns `None` for an empty image or one over [`MAX_SIDE`], which the
/// caller treats as "no drag image" rather than as an error.
pub(crate) fn layout(width: u32, height: u32, scale: u32, hotspot: Point) -> Option<(i32, i32)> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let scale = scale.max(1) as f32;
    let pixel = |v: f32, side: u32| -> i32 {
        let v = if v.is_finite() { v * scale } else { 0.0 };
        v.round().clamp(0.0, (side - 1) as f32) as i32
    };
    Some((pixel(hotspot.x, width), pixel(hotspot.y, height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_pixels_only_swap_red_and_blue() {
        assert_eq!(
            premultiplied_bgra(&[10, 20, 30, 255]),
            vec![30, 20, 10, 255]
        );
    }

    #[test]
    fn transparent_pixels_lose_their_colour() {
        // A straight-alpha transparent pixel often keeps stale colour; left in
        // a premultiplied buffer it would add light instead of nothing.
        assert_eq!(premultiplied_bgra(&[200, 100, 50, 0]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn half_alpha_is_scaled_with_rounding() {
        // 255 * 128 / 255 = 128 exactly; 101 * 128 / 255 = 50.7 -> 51.
        assert_eq!(
            premultiplied_bgra(&[255, 101, 0, 128]),
            vec![0, 51, 128, 128]
        );
    }

    #[test]
    fn a_ragged_buffer_is_refused_not_truncated() {
        assert!(premultiplied_bgra(&[1, 2, 3]).is_empty());
    }

    #[test]
    fn the_hotspot_is_scaled_from_points_to_pixels() {
        assert_eq!(layout(128, 64, 2, Point::new(16.0, 8.0)), Some((32, 16)));
    }

    #[test]
    fn the_hotspot_never_leaves_the_bitmap() {
        assert_eq!(layout(10, 10, 1, Point::new(500.0, -3.0)), Some((9, 0)));
        assert_eq!(layout(10, 10, 1, Point::new(f32::NAN, 4.0)), Some((0, 4)));
    }

    #[test]
    fn an_empty_or_enormous_image_is_no_drag_image() {
        assert_eq!(layout(0, 10, 1, Point::ZERO), None);
        assert_eq!(layout(10, 0, 1, Point::ZERO), None);
        assert_eq!(layout(MAX_SIDE + 1, 10, 1, Point::ZERO), None);
        assert_eq!(layout(10, MAX_SIDE + 1, 1, Point::ZERO), None);
        assert!(layout(MAX_SIDE, MAX_SIDE, 1, Point::ZERO).is_some());
    }
}
