//! Jembatan DPI: poin logis ⇄ piksel fisik.
//!
//! Framework di atas backend hanya tahu poin logis (`rustui_paint::Size`).
//! Swapchain hanya tahu piksel fisik. [`SurfaceGeometry`] adalah satu-satunya
//! tempat konversinya terjadi, sehingga "DPI benar" punya satu titik
//! kebenaran yang bisa diuji tanpa GPU.

use rustui_paint::{Rect, Size};

/// Kotak scissor dalam **piksel fisik**, sudah dijamin berada di dalam surface.
///
/// Tipe tersendiri (bukan `[u32; 4]` telanjang) supaya jaminan itu ikut
/// terbawa: satu-satunya cara membuatnya adalah lewat
/// [`SurfaceGeometry::scissor`], yang membulatkan ke luar dan menjepit ke
/// batas surface. Scissor di luar batas adalah *validation error* wgpu, jadi
/// jaminan ini bukan kerapian belaka.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScissorRect {
    /// Tepi kiri, piksel fisik.
    pub x: u32,
    /// Tepi atas, piksel fisik.
    pub y: u32,
    /// Lebar, piksel fisik (selalu > 0).
    pub width: u32,
    /// Tinggi, piksel fisik (selalu > 0).
    pub height: u32,
}

/// Ukuran surface dalam piksel fisik plus scale factor window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceGeometry {
    width: u32,
    height: u32,
    scale_factor: f64,
}

impl SurfaceGeometry {
    /// Geometri dari ukuran **fisik** dan scale factor.
    ///
    /// `scale_factor` yang tidak masuk akal (nol, negatif, NaN, tak hingga)
    /// dinormalkan ke 1.0 — lebih baik tampil salah ukuran daripada membagi
    /// dengan nol saat window berpindah monitor.
    pub fn new(width: u32, height: u32, scale_factor: f64) -> Self {
        Self {
            width,
            height,
            scale_factor: sanitize_scale(scale_factor),
        }
    }

    /// Geometri dari ukuran **logis** — dibulatkan ke piksel fisik terdekat.
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

    /// Lebar dalam piksel fisik.
    pub fn physical_width(self) -> u32 {
        self.width
    }

    /// Tinggi dalam piksel fisik.
    pub fn physical_height(self) -> u32 {
        self.height
    }

    /// Scale factor window (2.0 di layar Retina).
    pub fn scale_factor(self) -> f64 {
        self.scale_factor
    }

    /// Ukuran dalam poin logis — bentuk yang dilihat layout dan widget.
    pub fn logical_size(self) -> Size {
        Size::new(
            (self.width as f64 / self.scale_factor) as f32,
            (self.height as f64 / self.scale_factor) as f32,
        )
    }

    /// Benar bila surface punya luas — window yang diminimalkan berukuran 0×0
    /// dan **tidak boleh** dikonfigurasi (wgpu menolak dimensi nol).
    pub fn is_renderable(self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Seluruh surface sebagai scissor — keadaan awal sebuah render pass.
    ///
    /// `None` bila surface tidak punya luas sama sekali.
    pub(crate) fn full_scissor(self) -> Option<ScissorRect> {
        (self.width > 0 && self.height > 0).then_some(ScissorRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        })
    }

    /// Kotak potong (**poin logis, absolut**) → scissor rect (piksel fisik).
    ///
    /// Tiga hal terjadi di sini, dan ketiganya wajib:
    ///
    /// 1. **Skala DPI**: clip datang dalam poin logis karena itulah satu-satunya
    ///    ruang koordinat yang dikenal layout dan `rustui-paint`; scissor
    ///    bekerja dalam piksel fisik.
    /// 2. **Pembulatan ke luar** (`floor` di tepi min, `ceil` di tepi max):
    ///    membulatkan ke dalam akan memakan satu piksel tepi konten pada scale
    ///    pecahan — kotak yang seharusnya pas dengan viewport jadi kehilangan
    ///    baris piksel terakhirnya. Scissor bukan alat anti-alias; tugasnya
    ///    hanya membuang yang jelas-jelas di luar.
    /// 3. **Penjepitan ke batas surface**: scissor yang melewati attachment
    ///    adalah validation error wgpu, bukan sekadar gambar yang salah.
    ///
    /// `None` berarti tidak ada satu piksel pun yang lolos — kotak kosong,
    /// terbalik, NaN, atau seluruhnya di luar surface. Pemanggil melewati
    /// batch itu sepenuhnya, bukan menggambarnya tanpa potong.
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

    /// Salinan dengan ukuran fisik baru (event `Resized`).
    pub fn with_physical_size(self, width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..self
        }
    }

    /// Salinan dengan scale factor baru (event `ScaleFactorChanged`:
    /// window pindah monitor, atau user mengubah skala tampilan).
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

/// Bulatkan sebuah tepi (sudah dalam piksel fisik) ke `0..=batas`.
///
/// NaN jatuh ke 0: tepi min jadi 0 dan tepi max jadi 0, sehingga kotaknya
/// kosong dan batch-nya dilewati — jauh lebih baik daripada `as u32` atas NaN.
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
        // 1.25 adalah kasus `wp_fractional_scale_v1` yang lazim di Linux.
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
        // winit mengirim ScaleFactorChanged lebih dulu, Resized menyusul —
        // ukuran fisik harus bertahan sampai event kedua datang.
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
        // Clip 10..110 poin di layar 2× harus menutup piksel 20..220.
        let g = SurfaceGeometry::new(512, 512, 2.0);
        assert_eq!(
            scissor(g, Rect::new(10.0, 5.0, 100.0, 50.0)),
            Some((20, 10, 200, 100))
        );
    }

    #[test]
    fn pembulatan_selalu_ke_luar_agar_tepi_konten_tidak_termakan() {
        // Scale pecahan (1,5): clip 10..15 poin jatuh di 15..22,5 px.
        // Membulatkan ke dalam (15..22) akan memakan piksel tepi konten yang
        // sah; ke luar (15..23) tidak pernah memotong terlalu banyak.
        let g = SurfaceGeometry::new(1000, 1000, 1.5);
        assert_eq!(
            scissor(g, Rect::new(10.0, 10.0, 5.0, 5.0)),
            Some((15, 15, 8, 8)),
            "min dibulatkan turun, max dibulatkan naik"
        );
        // Kotak setipis apa pun tetap menyisakan minimal satu piksel.
        let s = scissor(g, Rect::new(10.125, 10.125, 0.125, 0.125)).expect("tidak boleh hilang");
        assert!(s.2 >= 1 && s.3 >= 1, "{s:?}");
    }

    #[test]
    fn clip_yang_melewati_tepi_dijepit_ke_surface() {
        // Tanpa penjepitan ini wgpu menolak render pass-nya (validation error).
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
