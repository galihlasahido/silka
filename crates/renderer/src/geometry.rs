//! The DPI bridge: logical points ⇄ physical pixels.
//!
//! The framework above the backend only knows logical points
//! (`silka_paint::Size`). The swapchain only knows physical pixels.
//! [`SurfaceGeometry`] is the single place where the conversion happens, so
//! "correct DPI" has one source of truth that can be tested without a GPU.

use silka_paint::{Rect, Size};

/// A scissor rect in **physical pixels**, guaranteed to lie inside the surface.
///
/// It is its own type (rather than a bare `[u32; 4]`) so the guarantee travels
/// with it: the only way to build one is through [`SurfaceGeometry::scissor`],
/// which rounds outward and clamps to the surface bounds. An out-of-bounds
/// scissor is a wgpu *validation error*, so this guarantee is not mere
/// tidiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScissorRect {
    /// Left edge, physical pixels.
    pub x: u32,
    /// Top edge, physical pixels.
    pub y: u32,
    /// Width, physical pixels (always > 0).
    pub width: u32,
    /// Height, physical pixels (always > 0).
    pub height: u32,
}

/// The surface size in physical pixels, plus the window's scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceGeometry {
    width: u32,
    height: u32,
    scale_factor: f64,
}

impl SurfaceGeometry {
    /// Geometry from a **physical** size and a scale factor.
    ///
    /// A nonsensical `scale_factor` (zero, negative, NaN, infinite) is
    /// normalized to 1.0 — showing up at the wrong size beats dividing by zero
    /// when the window moves between monitors.
    pub fn new(width: u32, height: u32, scale_factor: f64) -> Self {
        Self {
            width,
            height,
            scale_factor: sanitize_scale(scale_factor),
        }
    }

    /// Geometry from a **logical** size — rounded to the nearest physical
    /// pixel.
    pub fn from_logical(size: Size, scale_factor: f64) -> Self {
        let scale = sanitize_scale(scale_factor);
        let w = (size.width.max(0.0) as f64 * scale).round() as u32;
        let h = (size.height.max(0.0) as f64 * scale).round() as u32;
        Self {
            width: w,
            height: h,
            scale_factor: scale,
        }
    }

    /// Width in physical pixels.
    pub fn physical_width(self) -> u32 {
        self.width
    }

    /// Height in physical pixels.
    pub fn physical_height(self) -> u32 {
        self.height
    }

    /// The window's scale factor (2.0 on a Retina display).
    pub fn scale_factor(self) -> f64 {
        self.scale_factor
    }

    /// The size in logical points — the form layout and widgets see.
    pub fn logical_size(self) -> Size {
        Size::new(
            (self.width as f64 / self.scale_factor) as f32,
            (self.height as f64 / self.scale_factor) as f32,
        )
    }

    /// True when the surface has any area — a minimized window is 0×0 and must
    /// **not** be configured (wgpu rejects zero dimensions).
    pub fn is_renderable(self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// The whole surface as a scissor — a render pass's initial state.
    ///
    /// `None` when the surface has no area at all.
    pub(crate) fn full_scissor(self) -> Option<ScissorRect> {
        (self.width > 0 && self.height > 0).then_some(ScissorRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        })
    }

    /// Clip rect (**absolute logical points**) → scissor rect (physical
    /// pixels).
    ///
    /// Three things happen here, and all three are required:
    ///
    /// 1. **DPI scaling**: clips arrive in logical points because that is the
    ///    only coordinate space layout and `silka-paint` know; scissors work in
    ///    physical pixels.
    /// 2. **Rounding outward** (`floor` on the min edges, `ceil` on the max
    ///    edges): rounding inward would eat one pixel off the content edge at
    ///    fractional scales — a rect meant to line up with the viewport would
    ///    lose its last row of pixels. A scissor is not an anti-aliasing tool;
    ///    its job is only to discard what is plainly outside.
    /// 3. **Clamping to the surface bounds**: a scissor that runs past the
    ///    attachment is a wgpu validation error, not merely a wrong picture.
    ///
    /// `None` means not a single pixel makes it through — an empty, inverted,
    /// or NaN rect, or one entirely outside the surface. The caller skips that
    /// batch entirely rather than drawing it unclipped.
    pub(crate) fn scissor(self, rect: Rect) -> Option<ScissorRect> {
        let s = self.scale_factor;
        let x0 = jepit((rect.min_x() as f64 * s).floor(), self.width);
        let y0 = jepit((rect.min_y() as f64 * s).floor(), self.height);
        let x1 = jepit((rect.max_x() as f64 * s).ceil(), self.width);
        let y1 = jepit((rect.max_y() as f64 * s).ceil(), self.height);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(ScissorRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }

    /// A copy with a new physical size (the `Resized` event).
    pub fn with_physical_size(self, width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..self
        }
    }

    /// A copy with a new scale factor (the `ScaleFactorChanged` event: the
    /// window moved to another monitor, or the user changed the display scale).
    pub fn with_scale_factor(self, scale_factor: f64) -> Self {
        Self {
            scale_factor: sanitize_scale(scale_factor),
            ..self
        }
    }
}

impl Default for SurfaceGeometry {
    fn default() -> Self {
        Self::new(0, 0, 1.0)
    }
}

/// Clamp one edge (already in physical pixels) into `0..=batas`.
///
/// NaN falls to 0: the min edge becomes 0 and the max edge becomes 0, so the
/// rect is empty and its batch is skipped — far better than `as u32` on a NaN.
fn jepit(v: f64, batas: u32) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        0
    } else if v >= batas as f64 {
        batas
    } else {
        v as u32
    }
}

fn sanitize_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_menggandakan_piksel_fisik() {
        let g = SurfaceGeometry::from_logical(Size::new(1024.0, 720.0), 2.0);
        assert_eq!((g.physical_width(), g.physical_height()), (2048, 1440));
        assert_eq!(g.logical_size(), Size::new(1024.0, 720.0));
    }

    #[test]
    fn scale_pecahan_wayland_dibulatkan() {
        // 1.25 is the common `wp_fractional_scale_v1` case on Linux.
        let g = SurfaceGeometry::from_logical(Size::new(801.0, 601.0), 1.25);
        assert_eq!((g.physical_width(), g.physical_height()), (1001, 751));
    }

    #[test]
    fn scale_tidak_masuk_akal_dinormalkan() {
        for buruk in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(SurfaceGeometry::new(100, 100, buruk).scale_factor(), 1.0);
        }
    }

    #[test]
    fn ukuran_nol_tidak_renderable() {
        assert!(!SurfaceGeometry::new(0, 720, 2.0).is_renderable());
        assert!(!SurfaceGeometry::new(1280, 0, 2.0).is_renderable());
        assert!(SurfaceGeometry::new(1, 1, 2.0).is_renderable());
    }

    #[test]
    fn ganti_scale_tidak_mengubah_piksel_fisik() {
        // winit sends ScaleFactorChanged first and Resized after — the physical
        // size must survive until that second event arrives.
        let g = SurfaceGeometry::new(2048, 1440, 2.0).with_scale_factor(1.0);
        assert_eq!(g.physical_width(), 2048);
        assert_eq!(g.logical_size(), Size::new(2048.0, 1440.0));
    }

    #[test]
    fn ganti_ukuran_tidak_mengubah_scale() {
        let g = SurfaceGeometry::new(100, 100, 2.0).with_physical_size(640, 480);
        assert_eq!(g.scale_factor(), 2.0);
        assert_eq!(g.logical_size(), Size::new(320.0, 240.0));
    }

    #[test]
    fn ukuran_logis_negatif_tidak_membuat_piksel_negatif() {
        let g = SurfaceGeometry::from_logical(Size::new(-10.0, -10.0), 2.0);
        assert_eq!((g.physical_width(), g.physical_height()), (0, 0));
        assert!(!g.is_renderable());
    }

    #[test]
    fn default_kosong_dan_scale_satu() {
        let g = SurfaceGeometry::default();
        assert!(!g.is_renderable());
        assert_eq!(g.scale_factor(), 1.0);
    }

    // ---- Scissor ---------------------------------------------------------

    fn scissor(g: SurfaceGeometry, rect: Rect) -> Option<(u32, u32, u32, u32)> {
        g.scissor(rect).map(|s| (s.x, s.y, s.width, s.height))
    }

    #[test]
    fn clip_logis_menjadi_piksel_fisik() {
        let g = SurfaceGeometry::new(256, 256, 1.0);
        assert_eq!(
            scissor(g, Rect::new(10.0, 20.0, 30.0, 40.0)),
            Some((10, 20, 30, 40))
        );
    }

    #[test]
    fn retina_menggandakan_scissor() {
        // A clip of 10..110 points on a 2× display must cover pixels 20..220.
        let g = SurfaceGeometry::new(512, 512, 2.0);
        assert_eq!(
            scissor(g, Rect::new(10.0, 5.0, 100.0, 50.0)),
            Some((20, 10, 200, 100))
        );
    }

    #[test]
    fn pembulatan_selalu_ke_luar_agar_tepi_konten_tidak_termakan() {
        // Fractional scale (1.5): a clip of 10..15 points lands on 15..22.5 px.
        // Rounding inward (15..22) would eat a legitimate content edge pixel;
        // rounding outward (15..23) never cuts too much.
        let g = SurfaceGeometry::new(1000, 1000, 1.5);
        assert_eq!(
            scissor(g, Rect::new(10.0, 10.0, 5.0, 5.0)),
            Some((15, 15, 8, 8)),
            "min dibulatkan turun, max dibulatkan naik"
        );
        // However thin the rect, at least one pixel always survives.
        let s = scissor(g, Rect::new(10.125, 10.125, 0.125, 0.125)).expect("tidak boleh hilang");
        assert!(s.2 >= 1 && s.3 >= 1, "{s:?}");
    }

    #[test]
    fn clip_yang_melewati_tepi_dijepit_ke_surface() {
        // Without this clamping wgpu rejects the render pass (validation error).
        let g = SurfaceGeometry::new(100, 80, 1.0);
        assert_eq!(
            scissor(g, Rect::new(-50.0, -50.0, 500.0, 500.0)),
            Some((0, 0, 100, 80))
        );
        assert_eq!(
            scissor(g, Rect::new(90.0, 70.0, 100.0, 100.0)),
            Some((90, 70, 10, 10))
        );
    }

    #[test]
    fn clip_seluruhnya_di_luar_surface_tidak_menghasilkan_scissor() {
        let g = SurfaceGeometry::new(100, 80, 1.0);
        assert_eq!(scissor(g, Rect::new(200.0, 0.0, 50.0, 50.0)), None);
        assert_eq!(scissor(g, Rect::new(0.0, -400.0, 50.0, 50.0)), None);
        assert_eq!(scissor(g, Rect::new(-10.0, 0.0, 5.0, 50.0)), None);
    }

    #[test]
    fn clip_kosong_atau_terbalik_tidak_menghasilkan_scissor() {
        let g = SurfaceGeometry::new(100, 80, 1.0);
        assert_eq!(scissor(g, Rect::new(10.0, 10.0, 0.0, 20.0)), None);
        assert_eq!(scissor(g, Rect::new(10.0, 10.0, 20.0, 0.0)), None);
        assert_eq!(scissor(g, Rect::new(10.0, 10.0, -20.0, -20.0)), None);
    }

    #[test]
    fn koordinat_ngawur_tidak_membuat_scissor_ngawur() {
        let g = SurfaceGeometry::new(100, 80, 1.0);
        for buruk in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let r = Rect::new(buruk, 0.0, 10.0, 10.0);
            let hasil = g.scissor(r);
            if let Some(s) = hasil {
                assert!(s.x + s.width <= 100 && s.y + s.height <= 80, "{s:?}");
            }
            let r = Rect::new(0.0, 0.0, buruk, buruk);
            if let Some(s) = g.scissor(r) {
                assert!(s.x + s.width <= 100 && s.y + s.height <= 80, "{s:?}");
            }
        }
    }

    #[test]
    fn scissor_penuh_adalah_seluruh_surface() {
        let g = SurfaceGeometry::new(64, 32, 2.0);
        let penuh = g.full_scissor().expect("surface punya luas");
        assert_eq!(
            (penuh.x, penuh.y, penuh.width, penuh.height),
            (0, 0, 64, 32)
        );
        assert_eq!(SurfaceGeometry::new(0, 0, 1.0).full_scissor(), None);
    }
}
