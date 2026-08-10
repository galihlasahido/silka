//! Restoring window geometry across runs (INTEGRASI-NATIVE §1, last row: "own
//! state" — no OS does this for us).
//!
//! Saving a rectangle is trivial. What is not trivial, and what this module is
//! actually about, is **the monitor that no longer exists**: a window saved on
//! a second display comes back on a machine that has been undocked, and a
//! naive restore puts it at `x = 2560` where nobody will ever see it again.
//! The rule here is therefore not "restore what was saved" but *"restore what
//! the user can still reach"* — and the fallback is to let the OS place the
//! window, which is always safe.
//!
//! Units are mixed on purpose, because the OS mixes them:
//!
//! - **Position is physical pixels**, because desktop coordinates are physical
//!   and a multi-monitor desktop has more than one scale factor in it.
//! - **Size is logical points**, so a window saved on a Retina display and
//!   reopened on a 1× monitor comes back the same *apparent* size instead of
//!   half of it.

use silka_paint::Size;

/// The smallest window that may ever be restored, in logical points — below
/// this a window is a dot the user cannot grab.
pub const MIN_RESTORED: Size = Size {
    width: 200.0,
    height: 120.0,
};

/// How much of a window has to remain on some monitor for the position to be
/// considered reachable, in logical points. Roughly "a grabbable strip of
/// title bar".
const MIN_VISIBLE_WIDTH: f32 = 120.0;
const MIN_VISIBLE_HEIGHT: f32 = 24.0;

/// A window's saved geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPlacement {
    /// Top-left corner of the window frame in **physical** desktop pixels.
    /// `None` means "the application never learned one" — the OS decides.
    pub position: Option<(i32, i32)>,
    /// Inner size in **logical points**.
    pub size: Size,
    /// The scale factor in effect when the geometry was saved. Needed to turn
    /// the logical size back into the physical rectangle the desktop
    /// coordinates are expressed in.
    pub scale: f64,
    /// Whether the window was maximized/zoomed.
    pub maximized: bool,
}

impl WindowPlacement {
    /// A placement with no position: size only, at scale 1.
    pub fn sized(size: Size) -> Self {
        Self {
            position: None,
            size,
            scale: 1.0,
            maximized: false,
        }
    }

    /// This placement with a position attached.
    pub fn at(mut self, x: i32, y: i32) -> Self {
        self.position = Some((x, y));
        self
    }

    /// This placement with a scale factor attached.
    pub fn scaled(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// This placement, maximized or not.
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    /// The window rectangle in physical pixels: `(x, y, width, height)`.
    ///
    /// The saved scale is used rather than the target monitor's, because the
    /// question being asked is "where did this rectangle used to be", and the
    /// answer has to be in the coordinate system it was saved in.
    pub fn physical_rect(&self) -> Option<(i64, i64, i64, i64)> {
        let (x, y) = self.position?;
        let scale = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        };
        let w = (self.size.width as f64 * scale).round().max(1.0) as i64;
        let h = (self.size.height as f64 * scale).round().max(1.0) as i64;
        Some((x as i64, y as i64, w, h))
    }
}

/// One monitor as the OS describes it, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorArea {
    /// Left edge in desktop coordinates.
    pub x: i32,
    /// Top edge in desktop coordinates.
    pub y: i32,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// The monitor's scale factor.
    pub scale: f64,
}

impl MonitorArea {
    /// A monitor at the origin with the given physical size and scale.
    pub fn new(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    /// The monitor rectangle: `(x, y, width, height)` in physical pixels.
    pub fn rect(&self) -> (i64, i64, i64, i64) {
        (
            self.x as i64,
            self.y as i64,
            self.width as i64,
            self.height as i64,
        )
    }

    /// The usable size in **logical points** — what a window size has to fit
    /// inside.
    pub fn logical_size(&self) -> Size {
        let scale = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        };
        Size::new(
            (self.width as f64 / scale) as f32,
            (self.height as f64 / scale) as f32,
        )
    }
}

/// The overlap of two rectangles, as `(width, height)` in physical pixels.
fn overlap(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> (i64, i64) {
    let w = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let h = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    (w.max(0), h.max(0))
}

/// The monitor a placement mostly sits on, if any.
pub fn monitor_for(placement: &WindowPlacement, monitors: &[MonitorArea]) -> Option<MonitorArea> {
    let rect = placement.physical_rect()?;
    monitors
        .iter()
        .map(|m| {
            let (w, h) = overlap(rect, m.rect());
            (w.saturating_mul(h), *m)
        })
        .filter(|(luas, _)| *luas > 0)
        .max_by_key(|(luas, _)| *luas)
        .map(|(_, m)| m)
}

/// Whether the user could still reach a window restored at this placement.
///
/// Two conditions, and both are failures seen in the wild:
///
/// 1. A grabbable strip has to remain on *some* monitor — otherwise the window
///    is on a display that has been unplugged.
/// 2. The window's top edge may not sit above the monitor's top edge, because
///    a title bar pushed off the top of the screen cannot be dragged back on
///    macOS, and lands under the taskbar on Windows.
pub fn is_reachable(placement: &WindowPlacement, monitors: &[MonitorArea]) -> bool {
    let Some(rect) = placement.physical_rect() else {
        return false;
    };
    monitors.iter().any(|m| {
        let (w, h) = overlap(rect, m.rect());
        let scale = if m.scale.is_finite() && m.scale > 0.0 {
            m.scale
        } else {
            1.0
        };
        let min_w = (MIN_VISIBLE_WIDTH as f64 * scale) as i64;
        let min_h = (MIN_VISIBLE_HEIGHT as f64 * scale) as i64;
        w >= min_w.min(rect.2) && h >= min_h.min(rect.3) && rect.1 >= m.y as i64
    })
}

/// Turn a saved placement into one that is safe to open **now**.
///
/// - The size is clamped to [`MIN_RESTORED`] and to what fits on the monitor
///   the window lands on (a 3440-wide window saved on an ultrawide must not
///   open wider than the laptop screen it comes back on).
/// - The position is kept only if it is still reachable; otherwise it is
///   dropped and the OS gets to place the window, which is the one placement
///   that is always correct.
/// - With **no monitors reported at all** (a headless CI run, a compositor
///   that will not say), the saved values are trusted as-is apart from the
///   minimum size: refusing to restore would be a worse guess than the user's
///   own last position.
pub fn restore_placement(saved: WindowPlacement, monitors: &[MonitorArea]) -> WindowPlacement {
    let mut out = saved;
    out.size = Size::new(
        out.size.width.max(MIN_RESTORED.width),
        out.size.height.max(MIN_RESTORED.height),
    );

    if monitors.is_empty() {
        return out;
    }

    // Shrink to the monitor the window is on, or to the largest one when the
    // position is already lost.
    let layar = monitor_for(&out, monitors).or_else(|| {
        monitors
            .iter()
            .copied()
            .max_by_key(|m| m.width as u64 * m.height as u64)
    });
    if let Some(layar) = layar {
        let muat = layar.logical_size();
        out.size = Size::new(
            out.size.width.min(muat.width).max(MIN_RESTORED.width),
            out.size.height.min(muat.height).max(MIN_RESTORED.height),
        );
    }

    if !is_reachable(&out, monitors) {
        out.position = None;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> MonitorArea {
        // A 13" Retina: 2560×1600 physical, 2× — 1280×800 points.
        MonitorArea::new(0, 0, 2560, 1600, 2.0)
    }

    fn eksternal() -> MonitorArea {
        // A 1× monitor to the right of the laptop.
        MonitorArea::new(2560, 0, 1920, 1080, 1.0)
    }

    #[test]
    fn posisi_yang_masih_terlihat_dipertahankan() {
        let saved = WindowPlacement::sized(Size::new(800.0, 600.0))
            .at(200, 100)
            .scaled(2.0);
        let out = restore_placement(saved, &[laptop()]);
        assert_eq!(out.position, Some((200, 100)));
        assert_eq!(out.size, Size::new(800.0, 600.0));
    }

    #[test]
    fn monitor_yang_dicabut_membuat_posisi_dilupakan() {
        // Saved on the external monitor; the user has since undocked.
        let saved = WindowPlacement::sized(Size::new(900.0, 600.0))
            .at(3000, 200)
            .scaled(1.0);
        let out = restore_placement(saved, &[laptop()]);
        assert_eq!(out.position, None, "window akan hilang di luar layar");
        // The size survives — only the position was unreachable.
        assert_eq!(out.size, Size::new(900.0, 600.0));
    }

    #[test]
    fn monitor_yang_masih_ada_membuat_posisi_dipertahankan() {
        let saved = WindowPlacement::sized(Size::new(900.0, 600.0))
            .at(3000, 200)
            .scaled(1.0);
        let out = restore_placement(saved, &[laptop(), eksternal()]);
        assert_eq!(out.position, Some((3000, 200)));
    }

    #[test]
    fn judul_di_atas_tepi_layar_tidak_bisa_diraih() {
        // A window whose title bar is above the screen top cannot be dragged
        // back down on macOS — restoring it is a trap.
        let saved = WindowPlacement::sized(Size::new(600.0, 400.0))
            .at(100, -80)
            .scaled(2.0);
        assert!(!is_reachable(&saved, &[laptop()]));
        assert_eq!(restore_placement(saved, &[laptop()]).position, None);
    }

    #[test]
    fn sisa_seiris_di_tepi_kanan_masih_dianggap_bisa_diraih() {
        // 300 physical px still on screen at 2× = 150 points wide: more than
        // the grab strip, so the user can pull it back.
        let saved = WindowPlacement::sized(Size::new(800.0, 600.0))
            .at(2560 - 300, 100)
            .scaled(2.0);
        assert!(is_reachable(&saved, &[laptop()]));

        // 100 physical px = 50 points: not enough to grab.
        let nyaris = WindowPlacement::sized(Size::new(800.0, 600.0))
            .at(2560 - 100, 100)
            .scaled(2.0);
        assert!(!is_reachable(&nyaris, &[laptop()]));
    }

    #[test]
    fn window_lebih_besar_dari_layar_dikecilkan() {
        // Saved on an ultrawide, restored on the laptop.
        let saved = WindowPlacement::sized(Size::new(3000.0, 1400.0))
            .at(10, 10)
            .scaled(1.0);
        let out = restore_placement(saved, &[laptop()]);
        let muat = laptop().logical_size();
        assert!(out.size.width <= muat.width, "{:?}", out.size);
        assert!(out.size.height <= muat.height, "{:?}", out.size);
    }

    #[test]
    fn window_terlalu_kecil_dinaikkan_ke_ukuran_minimum() {
        let saved = WindowPlacement::sized(Size::new(4.0, 2.0))
            .at(50, 50)
            .scaled(2.0);
        let out = restore_placement(saved, &[laptop()]);
        assert_eq!(out.size, MIN_RESTORED);
    }

    #[test]
    fn tanpa_daftar_monitor_nilai_tersimpan_dipercaya() {
        // Headless CI, or a compositor that refuses to enumerate: the user's
        // own last position is a better guess than ours.
        let saved = WindowPlacement::sized(Size::new(800.0, 600.0))
            .at(1234, 56)
            .scaled(2.0);
        let out = restore_placement(saved, &[]);
        assert_eq!(out.position, Some((1234, 56)));
        assert_eq!(out.size, Size::new(800.0, 600.0));
    }

    #[test]
    fn tanpa_posisi_tersimpan_os_yang_menempatkan() {
        let saved = WindowPlacement::sized(Size::new(800.0, 600.0));
        let out = restore_placement(saved, &[laptop()]);
        assert_eq!(out.position, None);
        assert_eq!(out.size, Size::new(800.0, 600.0));
        // …and the size still gets clamped to the monitor that exists.
        let out = restore_placement(
            WindowPlacement::sized(Size::new(9999.0, 9999.0)),
            &[laptop()],
        );
        assert!(out.size.width <= laptop().logical_size().width);
    }

    #[test]
    fn maximized_dibawa_apa_adanya() {
        let saved = WindowPlacement::sized(Size::new(800.0, 600.0))
            .at(0, 0)
            .scaled(2.0)
            .maximized(true);
        assert!(restore_placement(saved, &[laptop()]).maximized);
        assert!(!restore_placement(saved.maximized(false), &[laptop()]).maximized);
    }

    #[test]
    fn skala_menentukan_ukuran_fisik_bukan_logisnya() {
        // The same 800×600 points is 1600×1200 px on Retina and 800×600 on a
        // 1× monitor — the reachability check has to know which.
        let retina = WindowPlacement::sized(Size::new(800.0, 600.0))
            .at(0, 0)
            .scaled(2.0);
        assert_eq!(retina.physical_rect(), Some((0, 0, 1600, 1200)));
        let biasa = retina.scaled(1.0);
        assert_eq!(biasa.physical_rect(), Some((0, 0, 800, 600)));
    }

    #[test]
    fn skala_tidak_masuk_akal_diperlakukan_sebagai_satu() {
        let rusak = WindowPlacement::sized(Size::new(100.0, 100.0))
            .at(0, 0)
            .scaled(0.0);
        assert_eq!(rusak.physical_rect(), Some((0, 0, 100, 100)));
        let nan = rusak.scaled(f64::NAN);
        assert_eq!(nan.physical_rect(), Some((0, 0, 100, 100)));
        assert_eq!(
            MonitorArea::new(0, 0, 100, 100, 0.0).logical_size(),
            Size::new(100.0, 100.0)
        );
    }

    #[test]
    fn monitor_dengan_tumpang_tindih_terbesar_yang_dipilih() {
        let saved = WindowPlacement::sized(Size::new(400.0, 300.0))
            .at(2500, 100)
            .scaled(1.0);
        // 60 px on the laptop, 340 px on the external monitor.
        let m = monitor_for(&saved, &[laptop(), eksternal()]).expect("ada monitor");
        assert_eq!(m.x, eksternal().x);
    }

    #[test]
    fn window_di_antara_dua_monitor_tetap_bisa_diraih() {
        let saved = WindowPlacement::sized(Size::new(400.0, 300.0))
            .at(2400, 100)
            .scaled(1.0);
        assert!(is_reachable(&saved, &[laptop(), eksternal()]));
    }
}
