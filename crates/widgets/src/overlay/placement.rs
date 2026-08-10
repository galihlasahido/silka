//! Overlay placement geometry: **anchoring + auto-flip at the edges**.
//!
//! This is the part of the overlay system that touches neither the tree, nor
//! the GPU, nor time — purely `(panel size, anchor rect, screen bounds)` in,
//! `(position, side actually used)` out. That is why it can be tested to
//! exhaustion without a window (§9.5), and why every Tier 4 component in
//! `KOMPONEN.md` (dialog/popover/tooltip/menu/toast) shares one implementation
//! instead of each computing its own.
//!
//! The three rules [`place`] enforces:
//!
//! 1. **Logical sides, not physical ones.** [`Side::Start`]/[`Side::End`] are
//!    resolved through [`TextDirection`], so a menu that opens "towards the end
//!    of the line" opens to the left in an Arabic UI (§9.8). RTL mirroring is
//!    not an afterthought.
//! 2. **Auto-flip.** A panel that does not fit on the requested side moves to
//!    the opposite one — and when both sides are equally cramped it picks the
//!    one with more room, not the one that happens to be written first.
//! 3. **Shift, then clamp.** Once the side is settled, the panel is shifted
//!    along the cross axis to stay on screen, and as a second safety net both
//!    axes are clamped to the bounds. A panel **never** leaves the screen, no
//!    matter how badly sized it is.
//!
//! ```
//! use silka_core::tree::TextDirection;
//! use silka_paint::{Rect, Size};
//! use silka_widgets::overlay::{place, Placement, PhysicalSide, Side};
//!
//! // A button hugging the bottom edge: a popover "below" does not fit…
//! let layar = Rect::new(0.0, 0.0, 400.0, 300.0);
//! let tombol = Rect::new(100.0, 270.0, 80.0, 24.0);
//! let hasil = place(
//!     Size::new(200.0, 120.0),
//!     tombol,
//!     layar,
//!     Placement::anchored(Side::Bottom).gap(8.0),
//!     TextDirection::Ltr,
//! );
//! // …so it flips above the anchor all by itself.
//! assert_eq!(hasil.side, PhysicalSide::Top);
//! assert!(hasil.flipped);
//! ```

use silka_core::tree::{TextDirection, SPACING_UNIT};
use silka_paint::{Point, Rect, Size};

// ---------------------------------------------------------------------------
// Sides & alignment
// ---------------------------------------------------------------------------

/// The **logical** side of a placement — it follows the reading direction
/// (§9.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Side {
    /// Above the anchor (or hugging the layer's top edge).
    Top,
    /// Below the anchor (or hugging the layer's bottom edge).
    #[default]
    Bottom,
    /// Towards the start of the line: left in LTR, right in RTL.
    Start,
    /// Towards the end of the line: right in LTR, left in RTL.
    End,
}

impl Side {
    /// The physical side this resolves to under the `direction` reading order.
    pub fn resolve(self, direction: TextDirection) -> PhysicalSide {
        match (self, direction.is_rtl()) {
            (Side::Top, _) => PhysicalSide::Top,
            (Side::Bottom, _) => PhysicalSide::Bottom,
            (Side::Start, false) | (Side::End, true) => PhysicalSide::Left,
            (Side::End, false) | (Side::Start, true) => PhysicalSide::Right,
        }
    }
}

/// The resolved **physical** side — this is what the geometry works with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PhysicalSide {
    /// Above.
    Top,
    /// Below.
    #[default]
    Bottom,
    /// To the left.
    Left,
    /// To the right.
    Right,
}

impl PhysicalSide {
    /// The opposite side — where auto-flip goes.
    pub fn opposite(self) -> Self {
        match self {
            PhysicalSide::Top => PhysicalSide::Bottom,
            PhysicalSide::Bottom => PhysicalSide::Top,
            PhysicalSide::Left => PhysicalSide::Right,
            PhysicalSide::Right => PhysicalSide::Left,
        }
    }

    /// True if its main axis is vertical (top/bottom).
    pub fn is_vertical(self) -> bool {
        matches!(self, PhysicalSide::Top | PhysicalSide::Bottom)
    }

    /// A short name for debugging and golden tests.
    pub const fn name(self) -> &'static str {
        match self {
            PhysicalSide::Top => "top",
            PhysicalSide::Bottom => "bottom",
            PhysicalSide::Left => "left",
            PhysicalSide::Right => "right",
        }
    }
}

/// How the panel is aligned on the **cross axis** of the side in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// Start-aligned (left/top; right/top under RTL for vertical sides).
    Start,
    /// Center-aligned.
    #[default]
    Center,
    /// End-aligned.
    End,
}

impl Align {
    /// The mirrored alignment — used under an RTL reading direction.
    pub fn mirrored(self) -> Self {
        match self {
            Align::Start => Align::End,
            Align::Center => Align::Center,
            Align::End => Align::Start,
        }
    }
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/// How an overlay positions itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlacementMode {
    /// Centered in the layer, without an anchor — dialogs/alerts.
    #[default]
    Center,
    /// Attached outside the anchor rect — popovers, menus, tooltips.
    Anchored,
    /// Attached **inside** an edge of the layer — sheets, drawers, toasts.
    Edge,
}

/// The full placement recipe: mode, side, alignment, gap, and permission to
/// flip/shift.
///
/// Written Dart-style (§2.5): a constructor function + method chaining.
///
/// ```
/// use silka_widgets::overlay::{Align, Placement, Side};
///
/// // A menu opening downwards, aligned to the line start, 4pt away.
/// let _ = Placement::anchored(Side::Bottom).align(Align::Start).gap(4.0);
/// // A toast in the bottom-end corner with a 16pt margin.
/// let _ = Placement::edge(Side::Bottom).align(Align::End).gap(16.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The placement mode.
    pub mode: PlacementMode,
    /// The requested logical side.
    pub side: Side,
    /// Alignment on the cross axis.
    pub align: Align,
    /// Distance to the anchor ([`PlacementMode::Anchored`]) or to the layer
    /// edge ([`PlacementMode::Edge`]) — **always** from the theme spacing scale.
    pub gap: f32,
    /// May flip to the opposite side when it does not fit.
    pub flip: bool,
    /// May be shifted along the cross axis to stay on screen.
    pub shift: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self::center()
    }
}

impl Placement {
    /// Centered in the layer — a modal dialog.
    pub fn center() -> Self {
        Self {
            mode: PlacementMode::Center,
            side: Side::Top,
            align: Align::Center,
            gap: 0.0,
            flip: false,
            shift: true,
        }
    }

    /// Attached to the anchor on `side` — popover/menu/tooltip.
    pub fn anchored(side: Side) -> Self {
        Self {
            mode: PlacementMode::Anchored,
            side,
            align: Align::Center,
            gap: SPACING_UNIT,
            flip: true,
            shift: true,
        }
    }

    /// Attached to the layer edge on `side` — sheet/drawer/toast.
    pub fn edge(side: Side) -> Self {
        Self {
            mode: PlacementMode::Edge,
            side,
            align: Align::Center,
            gap: 0.0,
            flip: false,
            shift: true,
        }
    }

    /// Alignment on the cross axis.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Distance to the anchor/edge, in logical points (spacing token).
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// Allow/forbid auto-flip.
    pub fn flip(mut self, flip: bool) -> Self {
        self.flip = flip;
        self
    }

    /// Allow/forbid cross-axis shifting.
    pub fn shift(mut self, shift: bool) -> Self {
        self.shift = shift;
        self
    }

    /// The default enter-transition travel distance for a panel of size `panel`.
    ///
    /// [`PlacementMode::Edge`] comes in from off-screen, so its distance is the
    /// size of the panel itself; everything else only needs to "emerge" by two
    /// steps of the spacing scale (§2.6) — enough to read as movement, not
    /// enough to feel sluggish.
    pub fn default_travel(self, panel: Size) -> f32 {
        match self.mode {
            PlacementMode::Edge => {
                let main = if self.side.resolve(TextDirection::Ltr).is_vertical() {
                    panel.height
                } else {
                    panel.width
                };
                main + self.gap
            }
            _ => SPACING_UNIT * 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// The result of one placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// The panel's top-left corner, in the same coordinates as `bounds`.
    pub origin: Point,
    /// The physical side **ultimately** used (after auto-flip).
    pub side: PhysicalSide,
    /// The mode that produced it — it decides the enter-transition direction.
    pub mode: PlacementMode,
    /// True if auto-flip changed the side away from the one requested.
    pub flipped: bool,
    /// The cross-axis shift applied to keep the panel on screen.
    pub shifted: f32,
}

impl Placed {
    /// The rect of a `panel`-sized panel at this position.
    pub fn rect(self, panel: Size) -> Rect {
        Rect::from_origin_size(self.origin, panel)
    }

    /// The enter-transition offset at `progress` (0 = closed, 1 = open).
    ///
    /// Its direction follows the side, and that is what makes the motion
    /// legible:
    ///
    /// - **Anchored/Center** emerges *from* the anchor — a popover below a
    ///   button starts slightly higher and settles down into place (the same
    ///   pattern AppKit and Radix use).
    /// - **Edge** comes in *from off* screen — a sheet from the top really does
    ///   descend from the top edge rather than merely nudging into place.
    ///
    /// `distance` comes from [`Placement::default_travel`] or from a spacing
    /// token the caller picked; no number is born here.
    pub fn enter_offset(self, distance: f32, progress: f32) -> Point {
        let sisa = distance * (1.0 - progress.clamp(0.0, 1.0));
        if sisa == 0.0 {
            return Point::ZERO;
        }
        let keluar = matches!(self.mode, PlacementMode::Edge);
        let arah = match self.side {
            PhysicalSide::Top => Point::new(0.0, 1.0),
            PhysicalSide::Bottom => Point::new(0.0, -1.0),
            PhysicalSide::Left => Point::new(1.0, 0.0),
            PhysicalSide::Right => Point::new(-1.0, 0.0),
        };
        let tanda = if keluar { -sisa } else { sisa };
        Point::new(arah.x * tanda, arah.y * tanda)
    }
}

// ---------------------------------------------------------------------------
// Anchor
// ---------------------------------------------------------------------------

/// An overlay's anchor point, in **layer-local** coordinates.
///
/// Deliberately data rather than a `NodeId`: a render node must never peek at
/// another node's geometry from inside its own layout (the rule that "a node
/// never knows its own position", `silka_core::tree`). Translating a trigger
/// node into a rect is the job of [`crate::overlay::anchor_rect`], called
/// outside layout — usually in the handler that opens the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Anchor {
    /// No anchor — [`PlacementMode::Anchored`] falls back to the layer center.
    #[default]
    None,
    /// The trigger's rect (a button, a menu row).
    Rect(Rect),
    /// A single point — a context menu at the cursor position.
    Point(Point),
}

impl Anchor {
    /// The effective anchor rect; `bounds` serves as the last-resort fallback.
    pub fn rect(self, bounds: Rect) -> Rect {
        match self {
            Anchor::Rect(r) => r,
            Anchor::Point(p) => Rect::from_origin_size(p, Size::ZERO),
            Anchor::None => Rect::from_origin_size(bounds.center(), Size::ZERO),
        }
    }

    /// True if there really is an anchor.
    pub fn is_some(self) -> bool {
        !matches!(self, Anchor::None)
    }
}

// ---------------------------------------------------------------------------
// place()
// ---------------------------------------------------------------------------

/// Place a `panel`-sized panel relative to `anchor` inside `bounds`.
///
/// All coordinates live in the same space (layer-local). The result is
/// **always** inside `bounds` as long as the panel fits; when it does not, it
/// is pinned to the start edge — a clipped panel can still be read, a panel
/// lost off-screen cannot.
pub fn place(
    panel: Size,
    anchor: Rect,
    bounds: Rect,
    placement: Placement,
    direction: TextDirection,
) -> Placed {
    match placement.mode {
        PlacementMode::Center => pusat(panel, bounds, placement),
        PlacementMode::Edge => tepi(panel, bounds, placement, direction),
        PlacementMode::Anchored => tertambat(panel, anchor, bounds, placement, direction),
    }
}

fn pusat(panel: Size, bounds: Rect, placement: Placement) -> Placed {
    let tengah = bounds.center();
    let x = jepit(
        tengah.x - panel.width * 0.5,
        bounds.min_x(),
        bounds.max_x(),
        panel.width,
    );
    let y = jepit(
        tengah.y - panel.height * 0.5,
        bounds.min_y(),
        bounds.max_y(),
        panel.height,
    );
    Placed {
        origin: Point::new(x, y),
        // A dialog rises into place: side "Top" means upward motion.
        side: PhysicalSide::Top,
        mode: placement.mode,
        flipped: false,
        shifted: 0.0,
    }
}

fn tepi(panel: Size, bounds: Rect, placement: Placement, direction: TextDirection) -> Placed {
    let sisi = placement.side.resolve(direction);
    let m = placement.gap;
    let utama = match sisi {
        PhysicalSide::Top => bounds.min_y() + m,
        PhysicalSide::Bottom => bounds.max_y() - m - panel.height,
        PhysicalSide::Left => bounds.min_x() + m,
        PhysicalSide::Right => bounds.max_x() - m - panel.width,
    };
    let (silang_min, silang_max, panel_silang) = if sisi.is_vertical() {
        (bounds.min_x(), bounds.max_x(), panel.width)
    } else {
        (bounds.min_y(), bounds.max_y(), panel.height)
    };
    let align = perataan_efektif(placement.align, sisi, direction);
    let silang = match align {
        Align::Start => silang_min + m,
        Align::Center => (silang_min + silang_max) * 0.5 - panel_silang * 0.5,
        Align::End => silang_max - m - panel_silang,
    };
    rakit(panel, bounds, placement, sisi, utama, silang, false, 0.0)
}

fn tertambat(
    panel: Size,
    anchor: Rect,
    bounds: Rect,
    placement: Placement,
    direction: TextDirection,
) -> Placed {
    let diminta = placement.side.resolve(direction);
    let utama_di = |s: PhysicalSide| match s {
        PhysicalSide::Top => anchor.min_y() - placement.gap - panel.height,
        PhysicalSide::Bottom => anchor.max_y() + placement.gap,
        PhysicalSide::Left => anchor.min_x() - placement.gap - panel.width,
        PhysicalSide::Right => anchor.max_x() + placement.gap,
    };
    let muat = |s: PhysicalSide| match s {
        PhysicalSide::Top => utama_di(s) >= bounds.min_y(),
        PhysicalSide::Bottom => utama_di(s) + panel.height <= bounds.max_y(),
        PhysicalSide::Left => utama_di(s) >= bounds.min_x(),
        PhysicalSide::Right => utama_di(s) + panel.width <= bounds.max_x(),
    };
    // The free space beyond the anchor on that side — the tiebreaker when
    // neither side fits.
    let ruang = |s: PhysicalSide| match s {
        PhysicalSide::Top => anchor.min_y() - bounds.min_y(),
        PhysicalSide::Bottom => bounds.max_y() - anchor.max_y(),
        PhysicalSide::Left => anchor.min_x() - bounds.min_x(),
        PhysicalSide::Right => bounds.max_x() - anchor.max_x(),
    };

    let sisi = if placement.flip && !muat(diminta) {
        let lawan = diminta.opposite();
        if muat(lawan) || ruang(lawan) > ruang(diminta) {
            lawan
        } else {
            diminta
        }
    } else {
        diminta
    };
    let flipped = sisi != diminta;

    let (silang_min, silang_max, anchor_min, anchor_max, panel_silang) = if sisi.is_vertical() {
        (
            bounds.min_x(),
            bounds.max_x(),
            anchor.min_x(),
            anchor.max_x(),
            panel.width,
        )
    } else {
        (
            bounds.min_y(),
            bounds.max_y(),
            anchor.min_y(),
            anchor.max_y(),
            panel.height,
        )
    };
    let align = perataan_efektif(placement.align, sisi, direction);
    let silang = match align {
        Align::Start => anchor_min,
        Align::Center => (anchor_min + anchor_max) * 0.5 - panel_silang * 0.5,
        Align::End => anchor_max - panel_silang,
    };
    let silang_akhir = if placement.shift {
        jepit(silang, silang_min, silang_max, panel_silang)
    } else {
        silang
    };
    rakit(
        panel,
        bounds,
        placement,
        sisi,
        utama_di(sisi),
        silang_akhir,
        flipped,
        silang_akhir - silang,
    )
}

/// Clamp the main axis, then assemble the result.
///
/// The main axis is **always** clamped, even when `shift` is off: `shift`
/// governs the cross axis (alignment against the anchor), whereas keeping the
/// panel on screen is a safety net that must not be switchable.
#[allow(clippy::too_many_arguments)]
fn rakit(
    panel: Size,
    bounds: Rect,
    placement: Placement,
    sisi: PhysicalSide,
    utama: f32,
    silang: f32,
    flipped: bool,
    shifted: f32,
) -> Placed {
    let (utama_min, utama_max, panel_utama) = if sisi.is_vertical() {
        (bounds.min_y(), bounds.max_y(), panel.height)
    } else {
        (bounds.min_x(), bounds.max_x(), panel.width)
    };
    let utama = jepit(utama, utama_min, utama_max, panel_utama);
    let (silang_min, silang_max, panel_silang) = if sisi.is_vertical() {
        (bounds.min_x(), bounds.max_x(), panel.width)
    } else {
        (bounds.min_y(), bounds.max_y(), panel.height)
    };
    let silang = jepit(silang, silang_min, silang_max, panel_silang);
    let origin = if sisi.is_vertical() {
        Point::new(silang, utama)
    } else {
        Point::new(utama, silang)
    };
    Placed {
        origin,
        side: sisi,
        mode: placement.mode,
        flipped,
        shifted,
    }
}

/// The alignment after RTL mirroring.
///
/// Only vertical sides get mirrored: their cross axis is horizontal, and
/// horizontal is the only axis that has a reading direction (§9.8).
fn perataan_efektif(align: Align, sisi: PhysicalSide, direction: TextDirection) -> Align {
    if sisi.is_vertical() && direction.is_rtl() {
        align.mirrored()
    } else {
        align
    }
}

/// Clamp `v` to `[min, max - size]`; if it does not fit, pin it to `min`.
fn jepit(v: f32, min: f32, max: f32, size: f32) -> f32 {
    if !v.is_finite() {
        return min;
    }
    let batas = max - size;
    if batas <= min {
        min
    } else {
        v.clamp(min, batas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYAR: Rect = Rect::new(0.0, 0.0, 400.0, 300.0);

    fn di_bawah() -> Placement {
        Placement::anchored(Side::Bottom).gap(8.0)
    }

    #[test]
    fn tertambat_di_bawah_saat_muat() {
        let anchor = Rect::new(100.0, 50.0, 80.0, 24.0);
        let hasil = place(
            Size::new(200.0, 120.0),
            anchor,
            LAYAR,
            di_bawah(),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.side, PhysicalSide::Bottom);
        assert!(!hasil.flipped);
        // 50 + 24 + gap 8 = 82; centered on the anchor: 140 - 100 = 40.
        assert_eq!(hasil.origin, Point::new(40.0, 82.0));
    }

    #[test]
    fn auto_flip_saat_sisi_yang_diminta_tidak_muat() {
        let anchor = Rect::new(100.0, 270.0, 80.0, 24.0);
        let hasil = place(
            Size::new(200.0, 120.0),
            anchor,
            LAYAR,
            di_bawah(),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.side, PhysicalSide::Top);
        assert!(hasil.flipped);
        // 270 - 8 - 120 = 142.
        assert_eq!(hasil.origin.y, 142.0);
    }

    #[test]
    fn flip_bisa_dimatikan() {
        let anchor = Rect::new(100.0, 270.0, 80.0, 24.0);
        let hasil = place(
            Size::new(200.0, 120.0),
            anchor,
            LAYAR,
            di_bawah().flip(false),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.side, PhysicalSide::Bottom);
        assert!(!hasil.flipped);
        // Still clamped onto the screen even with flipping turned off.
        assert_eq!(hasil.origin.y, 180.0);
    }

    #[test]
    fn dua_sisi_sempit_memilih_yang_ruangnya_lebih_besar() {
        // Anchor near the top: room below (300-60=240) > room above (36).
        let anchor = Rect::new(0.0, 36.0, 40.0, 24.0);
        let hasil = place(
            Size::new(100.0, 280.0),
            anchor,
            LAYAR,
            Placement::anchored(Side::Top).gap(0.0),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.side, PhysicalSide::Bottom);
        assert!(hasil.flipped);
    }

    #[test]
    fn digeser_agar_tetap_di_dalam_layar() {
        // Anchor flush right: a centered panel would overshoot the edge.
        let anchor = Rect::new(380.0, 50.0, 20.0, 24.0);
        let hasil = place(
            Size::new(200.0, 100.0),
            anchor,
            LAYAR,
            di_bawah(),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.origin.x, 200.0, "harus mentok tepi kanan");
        assert!(hasil.shifted != 0.0, "geserannya harus dilaporkan");
        assert!(hasil.origin.x + 200.0 <= LAYAR.max_x());
    }

    #[test]
    fn shift_bisa_dimatikan_tanpa_membuang_jaring_pengaman() {
        let anchor = Rect::new(380.0, 50.0, 20.0, 24.0);
        let hasil = place(
            Size::new(200.0, 100.0),
            anchor,
            LAYAR,
            di_bawah().shift(false),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.shifted, 0.0, "tidak ada geseran yang dilaporkan");
        // …but the panel still must not leave the screen.
        assert!(hasil.origin.x + 200.0 <= LAYAR.max_x());
    }

    #[test]
    fn panel_lebih_besar_dari_layar_dipatok_ke_tepi_awal() {
        let hasil = place(
            Size::new(900.0, 900.0),
            Rect::new(10.0, 10.0, 10.0, 10.0),
            LAYAR,
            di_bawah(),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.origin, Point::ZERO);
    }

    #[test]
    fn sisi_logis_tercermin_di_rtl() {
        assert_eq!(Side::Start.resolve(TextDirection::Ltr), PhysicalSide::Left);
        assert_eq!(Side::Start.resolve(TextDirection::Rtl), PhysicalSide::Right);
        assert_eq!(Side::End.resolve(TextDirection::Rtl), PhysicalSide::Left);
        // Vertical sides have no reading direction.
        assert_eq!(Side::Top.resolve(TextDirection::Rtl), PhysicalSide::Top);
    }

    #[test]
    fn perataan_ikut_tercermin_di_rtl() {
        let anchor = Rect::new(100.0, 50.0, 80.0, 24.0);
        let p = di_bawah().align(Align::Start);
        let ltr = place(Size::new(40.0, 20.0), anchor, LAYAR, p, TextDirection::Ltr);
        let rtl = place(Size::new(40.0, 20.0), anchor, LAYAR, p, TextDirection::Rtl);
        assert_eq!(ltr.origin.x, 100.0, "LTR: rata tepi kiri jangkar");
        assert_eq!(rtl.origin.x, 140.0, "RTL: rata tepi kanan jangkar");
    }

    #[test]
    fn tengah_mengabaikan_jangkar() {
        let hasil = place(
            Size::new(200.0, 100.0),
            Rect::new(0.0, 0.0, 10.0, 10.0),
            LAYAR,
            Placement::center(),
            TextDirection::Ltr,
        );
        assert_eq!(hasil.origin, Point::new(100.0, 100.0));
        assert_eq!(hasil.mode, PlacementMode::Center);
    }

    #[test]
    fn tepi_menempel_di_dalam_layer() {
        let hasil = place(
            Size::new(120.0, 60.0),
            Rect::default(),
            LAYAR,
            Placement::edge(Side::Bottom).align(Align::End).gap(16.0),
            TextDirection::Ltr,
        );
        // Bottom: 300 - 16 - 60 = 224. Line end (LTR = right): 400-16-120=264.
        assert_eq!(hasil.origin, Point::new(264.0, 224.0));
        assert_eq!(hasil.side, PhysicalSide::Bottom);
    }

    #[test]
    fn jangkar_kosong_jatuh_ke_tengah_layer() {
        let bounds = LAYAR;
        assert_eq!(Anchor::None.rect(bounds).origin, bounds.center());
        assert_eq!(
            Anchor::Point(Point::new(4.0, 5.0)).rect(bounds),
            Rect::new(4.0, 5.0, 0.0, 0.0)
        );
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(Anchor::Rect(r).rect(bounds), r);
    }

    #[test]
    fn transisi_masuk_menyembul_dari_jangkar() {
        let bawah = Placed {
            origin: Point::ZERO,
            side: PhysicalSide::Bottom,
            mode: PlacementMode::Anchored,
            flipped: false,
            shifted: 0.0,
        };
        // Closed: it starts above its resting spot (closer to the anchor).
        assert_eq!(bawah.enter_offset(10.0, 0.0), Point::new(0.0, -10.0));
        // Open: exactly at its resting spot.
        assert_eq!(bawah.enter_offset(10.0, 1.0), Point::ZERO);
        // Halfway through: half the distance.
        assert_eq!(bawah.enter_offset(10.0, 0.5), Point::new(0.0, -5.0));
    }

    #[test]
    fn transisi_tepi_masuk_dari_luar_layar() {
        let sheet = Placed {
            origin: Point::ZERO,
            side: PhysicalSide::Top,
            mode: PlacementMode::Edge,
            flipped: false,
            shifted: 0.0,
        };
        // A sheet from the top starts above the screen edge, not below it.
        assert_eq!(sheet.enter_offset(120.0, 0.0), Point::new(0.0, -120.0));
        assert_eq!(sheet.enter_offset(120.0, 1.0), Point::ZERO);
    }

    #[test]
    fn jarak_tempuh_bawaan_mengikuti_mode() {
        let panel = Size::new(200.0, 120.0);
        assert_eq!(
            Placement::anchored(Side::Bottom).default_travel(panel),
            SPACING_UNIT * 2.0
        );
        assert_eq!(
            Placement::center().default_travel(panel),
            SPACING_UNIT * 2.0
        );
        assert_eq!(
            Placement::edge(Side::Bottom)
                .gap(16.0)
                .default_travel(panel),
            136.0
        );
    }

    #[test]
    fn progress_di_luar_jangkauan_tidak_membuat_geseran_liar() {
        let p = Placed {
            origin: Point::ZERO,
            side: PhysicalSide::Bottom,
            mode: PlacementMode::Anchored,
            flipped: false,
            shifted: 0.0,
        };
        assert_eq!(p.enter_offset(10.0, 2.0), Point::ZERO);
        assert_eq!(p.enter_offset(10.0, -1.0), Point::new(0.0, -10.0));
    }
}
