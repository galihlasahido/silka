//! Geometri penempatan overlay: **anchor + auto-flip di tepi**.
//!
//! Ini bagian overlay yang tidak menyentuh pohon, tidak menyentuh GPU, dan
//! tidak menyentuh waktu — murni `(ukuran panel, kotak jangkar, batas layar)`
//! menjadi `(posisi, sisi yang akhirnya dipakai)`. Karena itu ia bisa diuji
//! habis-habisan tanpa window (§9.5), dan karena itu pula seluruh komponen
//! Tier 4 `KOMPONEN.md` (dialog/popover/tooltip/menu/toast) memakai satu
//! implementasi yang sama alih-alih masing-masing menghitung sendiri.
//!
//! Tiga aturan yang dijalankan [`place`]:
//!
//! 1. **Sisi logis, bukan fisik.** [`Side::Start`]/[`Side::End`] diresolusi
//!    lewat [`TextDirection`], jadi menu yang membuka "ke arah akhir baris"
//!    otomatis membuka ke kiri di antarmuka Arab (§9.8). Mirroring RTL bukan
//!    fitur susulan.
//! 2. **Auto-flip.** Panel yang tidak muat di sisi yang diminta pindah ke sisi
//!    seberang — dan kalau kedua sisi sama-sama sempit, ia memilih yang
//!    ruangnya lebih besar, bukan yang kebetulan ditulis lebih dulu.
//! 3. **Geser lalu jepit.** Setelah sisi ditentukan, panel digeser sepanjang
//!    sumbu silang agar tetap di dalam layar, dan sebagai jaring pengaman
//!    kedua sumbu dijepit ke batas. Panel **tidak pernah** keluar layar,
//!    seburuk apa pun ukurannya.
//!
//! ```
//! use rustui_core::tree::TextDirection;
//! use rustui_paint::{Rect, Size};
//! use rustui_widgets::overlay::{place, Placement, PhysicalSide, Side};
//!
//! // Tombol menempel di tepi bawah layar: popover "di bawah" tidak muat…
//! let layar = Rect::new(0.0, 0.0, 400.0, 300.0);
//! let tombol = Rect::new(100.0, 270.0, 80.0, 24.0);
//! let hasil = place(
//!     Size::new(200.0, 120.0),
//!     tombol,
//!     layar,
//!     Placement::anchored(Side::Bottom).gap(8.0),
//!     TextDirection::Ltr,
//! );
//! // …jadi ia membalik ke atas dengan sendirinya.
//! assert_eq!(hasil.side, PhysicalSide::Top);
//! assert!(hasil.flipped);
//! ```

use rustui_core::tree::{TextDirection, SPACING_UNIT};
use rustui_paint::{Point, Rect, Size};

// ---------------------------------------------------------------------------
// Sisi & perataan
// ---------------------------------------------------------------------------

/// Sisi **logis** sebuah penempatan — mengikuti arah baca (§9.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Side {
    /// Di atas jangkar (atau menempel tepi atas layer).
    Top,
    /// Di bawah jangkar (atau menempel tepi bawah layer).
    #[default]
    Bottom,
    /// Ke arah awal baris: kiri di LTR, kanan di RTL.
    Start,
    /// Ke arah akhir baris: kanan di LTR, kiri di RTL.
    End,
}

impl Side {
    /// Sisi fisik yang berlaku pada arah baca `direction`.
    pub fn resolve(self, direction: TextDirection) -> PhysicalSide {
        match (self, direction.is_rtl()) {
            (Side::Top, _) => PhysicalSide::Top,
            (Side::Bottom, _) => PhysicalSide::Bottom,
            (Side::Start, false) | (Side::End, true) => PhysicalSide::Left,
            (Side::End, false) | (Side::Start, true) => PhysicalSide::Right,
        }
    }
}

/// Sisi **fisik** hasil resolusi — inilah yang dipakai geometri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PhysicalSide {
    /// Di atas.
    Top,
    /// Di bawah.
    #[default]
    Bottom,
    /// Di kiri.
    Left,
    /// Di kanan.
    Right,
}

impl PhysicalSide {
    /// Sisi seberang — tujuan auto-flip.
    pub fn opposite(self) -> Self {
        match self {
            PhysicalSide::Top => PhysicalSide::Bottom,
            PhysicalSide::Bottom => PhysicalSide::Top,
            PhysicalSide::Left => PhysicalSide::Right,
            PhysicalSide::Right => PhysicalSide::Left,
        }
    }

    /// Benar bila sumbu utamanya vertikal (atas/bawah).
    pub fn is_vertical(self) -> bool {
        matches!(self, PhysicalSide::Top | PhysicalSide::Bottom)
    }

    /// Nama pendek untuk debug dan golden test.
    pub const fn name(self) -> &'static str {
        match self {
            PhysicalSide::Top => "top",
            PhysicalSide::Bottom => "bottom",
            PhysicalSide::Left => "left",
            PhysicalSide::Right => "right",
        }
    }
}

/// Perataan panel pada **sumbu silang** sisi yang dipakai.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// Rata awal (kiri/atas; kanan/atas di RTL untuk sisi vertikal).
    Start,
    /// Rata tengah.
    #[default]
    Center,
    /// Rata akhir.
    End,
}

impl Align {
    /// Perataan yang sudah dicerminkan — dipakai saat arah baca RTL.
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

/// Cara sebuah overlay menempatkan dirinya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlacementMode {
    /// Di tengah layer, tanpa jangkar — dialog/alert.
    #[default]
    Center,
    /// Menempel di luar kotak jangkar — popover, menu, tooltip.
    Anchored,
    /// Menempel di **dalam** tepi layer — sheet, drawer, toast.
    Edge,
}

/// Resep penempatan lengkap: mode, sisi, perataan, jarak, dan izin
/// flip/geser.
///
/// Ditulis gaya Dart (§2.5): fungsi konstruktor + method chaining.
///
/// ```
/// use rustui_widgets::overlay::{Align, Placement, Side};
///
/// // Menu yang membuka ke bawah, rata awal baris, berjarak 4pt.
/// let _ = Placement::anchored(Side::Bottom).align(Align::Start).gap(4.0);
/// // Toast di pojok bawah-akhir dengan margin 16pt.
/// let _ = Placement::edge(Side::Bottom).align(Align::End).gap(16.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Mode penempatan.
    pub mode: PlacementMode,
    /// Sisi logis yang diminta.
    pub side: Side,
    /// Perataan pada sumbu silang.
    pub align: Align,
    /// Jarak ke jangkar ([`PlacementMode::Anchored`]) atau ke tepi layer
    /// ([`PlacementMode::Edge`]) — **selalu** dari skala spacing theme.
    pub gap: f32,
    /// Boleh membalik ke sisi seberang saat tidak muat.
    pub flip: bool,
    /// Boleh digeser sepanjang sumbu silang agar tetap di dalam layar.
    pub shift: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self::center()
    }
}

impl Placement {
    /// Di tengah layer — dialog modal.
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

    /// Menempel pada jangkar di `side` — popover/menu/tooltip.
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

    /// Menempel pada tepi layer di `side` — sheet/drawer/toast.
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

    /// Perataan pada sumbu silang.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Jarak ke jangkar/tepi, poin logis (token spacing).
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// Izinkan/larang auto-flip.
    pub fn flip(mut self, flip: bool) -> Self {
        self.flip = flip;
        self
    }

    /// Izinkan/larang geser sumbu silang.
    pub fn shift(mut self, shift: bool) -> Self {
        self.shift = shift;
        self
    }

    /// Jarak tempuh bawaan transisi masuk untuk panel seukuran `panel`.
    ///
    /// [`PlacementMode::Edge`] muncul dari luar layar, jadi jaraknya seukuran
    /// panelnya sendiri; sisanya cukup "menyembul" dua langkah skala spacing
    /// (§2.6) — cukup untuk terbaca sebagai gerakan, tidak cukup untuk terasa
    /// lambat.
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
// Hasil
// ---------------------------------------------------------------------------

/// Hasil satu penempatan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Sudut kiri-atas panel, dalam koordinat yang sama dengan `bounds`.
    pub origin: Point,
    /// Sisi fisik yang **akhirnya** dipakai (sesudah auto-flip).
    pub side: PhysicalSide,
    /// Mode yang menghasilkannya — menentukan arah transisi masuk.
    pub mode: PlacementMode,
    /// Benar bila auto-flip mengubah sisi dari yang diminta.
    pub flipped: bool,
    /// Pergeseran sumbu silang yang dilakukan agar tetap di dalam layar.
    pub shifted: f32,
}

impl Placed {
    /// Kotak panel berukuran `panel` pada posisi ini.
    pub fn rect(self, panel: Size) -> Rect {
        Rect::from_origin_size(self.origin, panel)
    }

    /// Geseran transisi masuk pada `progress` (0 = tertutup, 1 = terbuka).
    ///
    /// Arahnya mengikuti sisi, dan itu yang membuat gerakannya terbaca:
    ///
    /// - **Anchored/Center** menyembul *dari* jangkar — popover di bawah
    ///   tombol mulai sedikit lebih tinggi lalu turun ke tempatnya (pola yang
    ///   sama dipakai AppKit dan Radix).
    /// - **Edge** masuk *dari luar* layar — sheet dari atas benar-benar turun
    ///   dari tepi atas, bukan sekadar bergeser sedikit.
    ///
    /// `distance` datang dari [`Placement::default_travel`] atau dari token
    /// spacing yang dipilih pemanggil; tidak ada angka yang lahir di sini.
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
// Jangkar
// ---------------------------------------------------------------------------

/// Titik tambat sebuah overlay, dalam koordinat **lokal layer**.
///
/// Sengaja data, bukan `NodeId`: sebuah render node tidak boleh mengintip
/// geometri node lain dari dalam layout-nya sendiri (aturan "node tidak pernah
/// tahu posisinya sendiri", `rustui_core::tree`). Yang menerjemahkan node
/// pemicu menjadi kotak adalah [`crate::overlay::anchor_rect`], dipanggil di
/// luar layout — biasanya di handler yang membuka overlay-nya.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Anchor {
    /// Tanpa jangkar — [`PlacementMode::Anchored`] jatuh ke tengah layer.
    #[default]
    None,
    /// Kotak pemicu (tombol, baris menu).
    Rect(Rect),
    /// Satu titik — menu konteks pada posisi kursor.
    Point(Point),
}

impl Anchor {
    /// Kotak jangkar yang berlaku; `bounds` dipakai sebagai jatuhan terakhir.
    pub fn rect(self, bounds: Rect) -> Rect {
        match self {
            Anchor::Rect(r) => r,
            Anchor::Point(p) => Rect::from_origin_size(p, Size::ZERO),
            Anchor::None => Rect::from_origin_size(bounds.center(), Size::ZERO),
        }
    }

    /// Benar bila jangkarnya benar-benar ada.
    pub fn is_some(self) -> bool {
        !matches!(self, Anchor::None)
    }
}

// ---------------------------------------------------------------------------
// place()
// ---------------------------------------------------------------------------

/// Tempatkan panel berukuran `panel` terhadap `anchor` di dalam `bounds`.
///
/// Seluruh koordinat berada di ruang yang sama (lokal layer). Hasilnya
/// **selalu** berada di dalam `bounds` selama panelnya muat; kalau tidak muat,
/// ia dipatok ke tepi awal — panel yang terpotong masih bisa dibaca, panel yang
/// hilang di luar layar tidak.
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
        // Dialog menyembul ke atas: sisi "Top" berarti gerakan naik.
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
    // Ruang kosong di luar jangkar pada sisi itu — penentu saat kedua sisi
    // sama-sama tidak muat.
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

/// Jepit sumbu utama lalu susun hasilnya.
///
/// Sumbu utama **selalu** dijepit, bahkan saat `shift` dimatikan: `shift`
/// mengatur sumbu silang (perataan terhadap jangkar), sedangkan menjaga panel
/// tetap di layar adalah jaring pengaman yang tidak boleh bisa dimatikan.
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

/// Perataan setelah mirroring RTL.
///
/// Hanya sisi vertikal yang ikut tercermin: sumbu silangnya horizontal, dan
/// horizontal-lah satu-satunya sumbu yang punya arah baca (§9.8).
fn perataan_efektif(align: Align, sisi: PhysicalSide, direction: TextDirection) -> Align {
    if sisi.is_vertical() && direction.is_rtl() {
        align.mirrored()
    } else {
        align
    }
}

/// Jepit `v` ke `[min, max - size]`; kalau tidak muat, patok ke `min`.
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
        // 50 + 24 + gap 8 = 82; rata tengah terhadap jangkar: 140 - 100 = 40.
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
        // Tetap dijepit ke dalam layar walau flip dimatikan.
        assert_eq!(hasil.origin.y, 180.0);
    }

    #[test]
    fn dua_sisi_sempit_memilih_yang_ruangnya_lebih_besar() {
        // Jangkar dekat atas: ruang di bawah (300-60=240) > ruang di atas (36).
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
        // Jangkar mepet kanan: panel rata tengah akan melewati tepi.
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
        // …tapi panel tetap tidak boleh keluar layar.
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
        // Sisi vertikal tidak punya arah baca.
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
        // Bawah: 300 - 16 - 60 = 224. Akhir baris (LTR = kanan): 400-16-120=264.
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
        // Tertutup: mulai di atas tempatnya (lebih dekat ke jangkar).
        assert_eq!(bawah.enter_offset(10.0, 0.0), Point::new(0.0, -10.0));
        // Terbuka: tepat di tempatnya.
        assert_eq!(bawah.enter_offset(10.0, 1.0), Point::ZERO);
        // Setengah jalan: setengah jarak.
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
        // Sheet dari atas mulai di atas tepi layar, bukan di bawahnya.
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
