//! Frame scheduling: **render hanya saat dirty** (REKOMENDASI §3.5).
//!
//! Modul ini murni logika — tidak tahu winit, tidak tahu wgpu, tidak tahu
//! CADisplayLink. Yang platform sediakan hanyalah dua hal:
//!
//! 1. **detak vsync** (kapan boleh menggambar), dan
//! 2. **interval vsync terukur** yang dipakai sebagai budget frame.
//!
//! Aturan yang mengikat: interval frame **tidak pernah dikonstanta**. Tidak ada
//! 16,6 ms di mana pun. Kalau platform tahu (CADisplayLink di macOS,
//! ProMotion-aware), nilainya datang dari sana; kalau tidak tahu, ia ditaksir
//! dari jarak antar-frame yang benar-benar terjadi ([`RefreshEstimator`]);
//! kalau belum cukup sampel, budget-nya **tidak diketahui** dan tidak ada yang
//! berpura-pura tahu.
//!
//! ```
//! use std::time::{Duration, Instant};
//! use rustui_core::scheduler::{Dirty, FrameScheduler, Wake};
//!
//! let mut s = FrameScheduler::new();
//! assert!(s.is_idle());                        // idle = benar-benar tidak menggambar
//! assert_eq!(s.request(Dirty::PAINT), Wake::Schedule);
//! assert_eq!(s.request(Dirty::LAYOUT), Wake::AlreadyScheduled);
//!
//! let t0 = Instant::now();
//! let mut start = s.begin_frame(t0);
//! assert!(start.reason().contains(Dirty::PAINT | Dirty::LAYOUT));
//! start.mark_built(t0 + Duration::from_millis(2));   // scene selesai dibangun
//! let timing = s.end_frame(start, t0 + Duration::from_millis(9), true);
//! assert_eq!(timing.build, Duration::from_millis(2));
//! assert_eq!(timing.present, Duration::from_millis(7));
//! assert!(s.is_idle());                        // kembali tidur
//! ```

use core::fmt;
use core::ops::{BitOr, BitOrAssign};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Dirty
// ---------------------------------------------------------------------------

/// Alasan sebuah frame dibutuhkan, sebagai bitset.
///
/// Scheduler tidak peduli isinya untuk memutuskan menggambar atau tidak —
/// "tidak kosong" sudah cukup. Nilainya dibawa sampai ke log frame supaya saat
/// menyelidiki jank kita tahu *siapa* yang membangunkan renderer.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Dirty(u8);

impl Dirty {
    /// Tidak ada yang perlu digambar.
    pub const NONE: Self = Self(0);
    /// Ukuran/posisi node berubah — layout harus dihitung ulang.
    pub const LAYOUT: Self = Self(1 << 0);
    /// Hanya tampilan yang berubah (warna, opacity) — layout tetap.
    pub const PAINT: Self = Self(1 << 1);
    /// Token theme berubah (dark mode OS, preset, accent color).
    pub const THEME: Self = Self(1 << 2);
    /// Surface berubah: resize, scale factor, atau swapchain ditata ulang.
    pub const SURFACE: Self = Self(1 << 3);
    /// Ada animasi/spring yang masih berjalan dan meminta frame berikutnya.
    pub const ANIMATION: Self = Self(1 << 4);
    /// Dibangunkan dari luar UI thread (hasil async, timer, IPC).
    pub const EXTERNAL: Self = Self(1 << 5);

    const NAMES: [(Self, &'static str); 6] = [
        (Self::LAYOUT, "layout"),
        (Self::PAINT, "paint"),
        (Self::THEME, "theme"),
        (Self::SURFACE, "surface"),
        (Self::ANIMATION, "animation"),
        (Self::EXTERNAL, "external"),
    ];

    /// Representasi bit mentah.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Benar bila tidak ada satu pun alasan tercatat.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Benar bila seluruh bit `other` ada di dalam `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Gabungan dua himpunan alasan.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Tambahkan alasan ke himpunan ini.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Kosongkan seluruh alasan.
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl BitOr for Dirty {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for Dirty {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl fmt::Debug for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("Dirty(none)");
        }
        f.write_str("Dirty(")?;
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("|")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

impl fmt::Display for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Vsync
// ---------------------------------------------------------------------------

/// Interval vsync tercepat yang dianggap masuk akal (1000 Hz).
pub const MIN_VSYNC_INTERVAL: Duration = Duration::from_micros(1_000);
/// Interval vsync terlambat yang dianggap masuk akal (10 Hz).
pub const MAX_VSYNC_INTERVAL: Duration = Duration::from_millis(100);

/// Dari mana angka interval vsync berasal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockSource {
    /// Belum ada informasi. **Tidak ada tebakan default** — lihat modul doc.
    Unknown,
    /// Ditaksir dari jarak antar-frame nyata ([`RefreshEstimator`]).
    Estimated,
    /// Dilaporkan langsung oleh OS (CADisplayLink macOS, compositor clock).
    DisplayLink,
}

impl ClockSource {
    /// Nama pendek untuk log.
    pub const fn label(self) -> &'static str {
        match self {
            ClockSource::Unknown => "unknown",
            ClockSource::Estimated => "estimated",
            ClockSource::DisplayLink => "display-link",
        }
    }

    /// Sumber yang lebih tepercaya menang saat keduanya tersedia.
    const fn trust(self) -> u8 {
        match self {
            ClockSource::Unknown => 0,
            ClockSource::Estimated => 1,
            ClockSource::DisplayLink => 2,
        }
    }
}

/// Detak layar: interval antar-vsync beserta asal-usulnya.
///
/// Sengaja tidak punya nilai default berupa angka. `Vsync::UNKNOWN` berarti
/// "belum tahu", dan [`Vsync::budget`] mengembalikan `None` — pemanggil harus
/// menangani ketidaktahuan itu, bukan menggantinya dengan 16,6 ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vsync {
    interval: Option<Duration>,
    source: ClockSource,
}

impl Vsync {
    /// Detak layar belum diketahui.
    pub const UNKNOWN: Self = Self {
        interval: None,
        source: ClockSource::Unknown,
    };

    /// Bangun dari interval terukur; `None` bila di luar rentang masuk akal.
    pub fn new(interval: Duration, source: ClockSource) -> Option<Self> {
        if !plausible(interval) {
            return None;
        }
        Some(Self {
            interval: Some(interval),
            source,
        })
    }

    /// Interval yang datang dari display link OS (paling tepercaya).
    pub fn display_link(interval: Duration) -> Option<Self> {
        Self::new(interval, ClockSource::DisplayLink)
    }

    /// Interval hasil taksiran dari jarak antar-frame nyata.
    pub fn estimated(interval: Duration) -> Option<Self> {
        Self::new(interval, ClockSource::Estimated)
    }

    /// Bangun dari laju refresh dalam hertz.
    pub fn from_hz(hz: f64, source: ClockSource) -> Option<Self> {
        if !hz.is_finite() || hz <= 0.0 {
            return None;
        }
        Self::new(Duration::from_secs_f64(1.0 / hz), source)
    }

    /// Interval antar-vsync, bila diketahui.
    pub fn interval(self) -> Option<Duration> {
        self.interval
    }

    /// Laju refresh dalam hertz, bila diketahui.
    pub fn hz(self) -> Option<f64> {
        self.interval.map(|d| 1.0 / d.as_secs_f64())
    }

    /// Asal-usul angka interval.
    pub fn source(self) -> ClockSource {
        self.source
    }

    /// Benar bila interval sudah diketahui.
    pub fn is_known(self) -> bool {
        self.interval.is_some()
    }

    /// Budget CPU untuk satu frame.
    ///
    /// Selalu turunan dari interval terukur — di ProMotion 120 Hz nilainya
    /// ~8,3 ms, di 60 Hz ~16,6 ms, dan di layar 240 Hz ~4,2 ms. Tidak pernah
    /// dikonstanta.
    pub fn budget(self) -> Option<Duration> {
        self.interval
    }

    /// Pilih yang lebih tepercaya; seri dimenangkan oleh `other` (lebih baru).
    fn preferred(self, other: Self) -> Self {
        if other.source.trust() >= self.source.trust() && other.is_known() {
            other
        } else if self.is_known() {
            self
        } else {
            other
        }
    }
}

impl Default for Vsync {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

impl fmt::Display for Vsync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.hz() {
            Some(hz) => write!(f, "{hz:.1} Hz ({})", self.source.label()),
            None => write!(f, "? Hz ({})", self.source.label()),
        }
    }
}

fn plausible(interval: Duration) -> bool {
    interval >= MIN_VSYNC_INTERVAL && interval <= MAX_VSYNC_INTERVAL
}

// ---------------------------------------------------------------------------
// RefreshEstimator
// ---------------------------------------------------------------------------

/// Kapasitas jendela sampel penaksir refresh rate.
const ESTIMATOR_CAPACITY: usize = 32;
/// Sampel minimum sebelum taksiran boleh dipercaya.
const ESTIMATOR_MIN_SAMPLES: usize = 8;

/// Penaksir interval vsync dari jarak antar-frame yang benar-benar terjadi.
///
/// Dipakai di platform yang tidak punya display link (fallback
/// `request_redraw` winit). Memakai **median**, bukan rata-rata: satu frame
/// yang di-drop tidak menggeser taksiran, dan jeda idle panjang langsung
/// ditolak karena di luar rentang [`MIN_VSYNC_INTERVAL`]..[`MAX_VSYNC_INTERVAL`].
#[derive(Debug, Clone)]
pub struct RefreshEstimator {
    samples: [Duration; ESTIMATOR_CAPACITY],
    len: usize,
    next: usize,
}

impl RefreshEstimator {
    /// Penaksir kosong.
    pub fn new() -> Self {
        Self {
            samples: [Duration::ZERO; ESTIMATOR_CAPACITY],
            len: 0,
            next: 0,
        }
    }

    /// Catat satu jarak antar-frame. Mengembalikan `false` bila ditolak.
    pub fn observe(&mut self, delta: Duration) -> bool {
        if !plausible(delta) {
            return false;
        }
        self.samples[self.next] = delta;
        self.next = (self.next + 1) % ESTIMATOR_CAPACITY;
        self.len = (self.len + 1).min(ESTIMATOR_CAPACITY);
        true
    }

    /// Jumlah sampel yang diterima (maksimum [`ESTIMATOR_CAPACITY`]).
    pub fn sample_count(&self) -> usize {
        self.len
    }

    /// Taksiran interval vsync, atau `None` bila sampelnya belum cukup.
    pub fn estimate(&self) -> Option<Duration> {
        if self.len < ESTIMATOR_MIN_SAMPLES {
            return None;
        }
        let mut buf = [Duration::ZERO; ESTIMATOR_CAPACITY];
        buf[..self.len].copy_from_slice(&self.samples[..self.len]);
        let slice = &mut buf[..self.len];
        slice.sort_unstable();
        Some(slice[self.len / 2])
    }

    /// Buang seluruh sampel (mis. window pindah monitor).
    pub fn reset(&mut self) {
        self.len = 0;
        self.next = 0;
    }
}

impl Default for RefreshEstimator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Statistik frame
// ---------------------------------------------------------------------------

/// Jumlah frame terakhir yang disimpan untuk persentil.
const STATS_WINDOW: usize = 120;

/// Hasil pengukuran satu frame.
///
/// Waktu frame sengaja dipecah dua, karena menggabungkannya menghasilkan angka
/// yang menipu:
///
/// - [`FrameTiming::build`] — kerja kita sendiri: membangun scene (view-diff,
///   layout, perintah paint). **Inilah yang dinilai terhadap budget vsync.**
/// - [`FrameTiming::present`] — menyerahkan scene ke backend. Angka ini
///   sebagian besar adalah *backpressure*: swapchain menahan pemanggil sampai
///   ada buffer bebas, jadi pada aplikasi yang sehat sekalipun ia mendekati
///   satu interval vsync. Menghitungnya sebagai "frame lambat" akan menuduh
///   setiap frame normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// Nomor urut frame sejak scheduler dibuat.
    pub index: u64,
    /// Alasan frame ini dijadwalkan (kosong = permintaan redraw dari OS).
    pub reason: Dirty,
    /// Waktu membangun scene — kerja CPU framework.
    pub build: Duration,
    /// Waktu menyerahkan scene ke backend, termasuk menunggu swapchain.
    pub present: Duration,
    /// Jarak ke frame sebelumnya; `None` untuk frame pertama.
    pub since_previous: Option<Duration>,
    /// Benar bila frame benar-benar dipresentasikan (bukan di-skip).
    pub presented: bool,
    /// Benar bila [`FrameTiming::build`] melewati budget vsync — hanya
    /// bermakna kalau interval vsync sudah diketahui.
    pub over_budget: bool,
}

impl FrameTiming {
    /// Total waktu dinding satu frame (`build + present`).
    pub fn total(&self) -> Duration {
        self.build + self.present
    }
}

/// Statistik bergulir waktu build frame.
#[derive(Debug, Clone)]
pub struct FrameStats {
    ring: [Duration; STATS_WINDOW],
    len: usize,
    next: usize,
    frames: u64,
    presented: u64,
    skipped: u64,
    over_budget: u64,
    worst: Duration,
    last: Option<FrameTiming>,
}

impl FrameStats {
    /// Statistik kosong.
    pub fn new() -> Self {
        Self {
            ring: [Duration::ZERO; STATS_WINDOW],
            len: 0,
            next: 0,
            frames: 0,
            presented: 0,
            skipped: 0,
            over_budget: 0,
            worst: Duration::ZERO,
            last: None,
        }
    }

    fn record(&mut self, timing: FrameTiming) {
        self.ring[self.next] = timing.build;
        self.next = (self.next + 1) % STATS_WINDOW;
        self.len = (self.len + 1).min(STATS_WINDOW);
        self.frames += 1;
        if timing.presented {
            self.presented += 1;
        } else {
            self.skipped += 1;
        }
        if timing.over_budget {
            self.over_budget += 1;
        }
        self.worst = self.worst.max(timing.build);
        self.last = Some(timing);
    }

    /// Total frame yang diukur.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Frame yang benar-benar dipresentasikan.
    pub fn presented(&self) -> u64 {
        self.presented
    }

    /// Frame yang dilewati (window minimal/tertutup/timeout swapchain).
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Frame yang melewati budget vsync.
    pub fn over_budget(&self) -> u64 {
        self.over_budget
    }

    /// Waktu build terlama sepanjang sesi.
    pub fn worst(&self) -> Duration {
        self.worst
    }

    /// Pengukuran frame terakhir.
    pub fn last(&self) -> Option<FrameTiming> {
        self.last
    }

    /// Rata-rata waktu build pada jendela terakhir.
    pub fn average(&self) -> Option<Duration> {
        if self.len == 0 {
            return None;
        }
        let total: Duration = self.ring[..self.len].iter().sum();
        Some(total / self.len as u32)
    }

    /// Persentil waktu build pada jendela terakhir (`p` dalam 0.0..=1.0).
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        if self.len == 0 || !(0.0..=1.0).contains(&p) {
            return None;
        }
        let mut buf = [Duration::ZERO; STATS_WINDOW];
        buf[..self.len].copy_from_slice(&self.ring[..self.len]);
        let slice = &mut buf[..self.len];
        slice.sort_unstable();
        let rank = (p * self.len as f64).ceil() as usize;
        Some(slice[rank.saturating_sub(1).min(self.len - 1)])
    }

    /// Median waktu build pada jendela terakhir.
    pub fn p50(&self) -> Option<Duration> {
        self.percentile(0.50)
    }

    /// Persentil 95 waktu build — angka yang menentukan "terasa mulus".
    pub fn p95(&self) -> Option<Duration> {
        self.percentile(0.95)
    }

    /// Ringkasan satu baris untuk log.
    pub fn summary(&self, vsync: Vsync) -> String {
        format!(
            "{} frame · build p50 {} · p95 {} · max {} · vsync {vsync} · budget {} · over-budget {}/{} · skipped {}",
            self.frames,
            opt_ms(self.p50()),
            opt_ms(self.p95()),
            ms(self.worst),
            opt_ms(vsync.budget()),
            self.over_budget,
            self.frames,
            self.skipped,
        )
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1_000.0)
}

fn opt_ms(d: Option<Duration>) -> String {
    match d {
        Some(d) => ms(d),
        None => "?".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// Perakit baris log frame time.
///
/// Sengaja mengembalikan `Option<String>` alih-alih mencetak sendiri: keputusan
/// "apakah ini debug build" milik pemanggil, dan formatnya jadi bisa diuji.
#[derive(Debug, Clone, Copy)]
pub struct FrameLogger {
    every: u64,
    warn_over_budget: bool,
}

impl FrameLogger {
    /// Logger yang mencetak ringkasan tiap `every` frame; `0` mematikan
    /// ringkasan berkala (hanya frame yang melewati budget yang dicatat).
    pub fn every(every: u64) -> Self {
        Self {
            every,
            warn_over_budget: true,
        }
    }

    /// Matikan peringatan frame yang melewati budget.
    pub fn quiet_over_budget(mut self) -> Self {
        self.warn_over_budget = false;
        self
    }

    /// Baris log untuk frame ini, atau `None` bila tidak perlu dicatat.
    pub fn line(&self, stats: &FrameStats, vsync: Vsync, timing: &FrameTiming) -> Option<String> {
        let lambat = self.warn_over_budget && timing.over_budget;
        let berkala = self.every > 0 && timing.index > 0 && timing.index % self.every == 0;
        if !lambat && !berkala {
            return None;
        }
        let tanda = if lambat { "LAMBAT " } else { "" };
        Some(format!(
            "rustui: {tanda}frame {} · build {} · present {} · Δ {} · sebab {} · vsync {vsync} · build p50 {} · p95 {} · max {} · over-budget {}/{}",
            timing.index,
            ms(timing.build),
            ms(timing.present),
            opt_ms(timing.since_previous),
            timing.reason,
            opt_ms(stats.p50()),
            opt_ms(stats.p95()),
            ms(stats.worst()),
            stats.over_budget(),
            stats.frames(),
        ))
    }
}

impl Default for FrameLogger {
    fn default() -> Self {
        Self::every(120)
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Apa yang harus platform lakukan setelah sebuah permintaan frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Bangunkan sumber vsync (unpause display link / `request_redraw`).
    Schedule,
    /// Frame sudah dijadwalkan — jangan poke platform dua kali.
    AlreadyScheduled,
    /// Tidak ada yang perlu digambar: window tersembunyi, atau `Dirty::NONE`.
    Suppressed,
}

/// Token satu frame yang sedang berjalan.
///
/// Dikembalikan [`FrameScheduler::begin_frame`] dan dikonsumsi
/// [`FrameScheduler::end_frame`] — bentuk ini membuat "lupa menutup frame"
/// menjadi kesalahan yang terlihat, bukan kebocoran diam-diam.
#[derive(Debug, Clone, Copy)]
pub struct FrameStart {
    index: u64,
    reason: Dirty,
    at: Instant,
    built_at: Option<Instant>,
}

impl FrameStart {
    /// Nomor urut frame ini.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Alasan frame ini dijadwalkan.
    pub fn reason(&self) -> Dirty {
        self.reason
    }

    /// Saat frame ini dimulai.
    pub fn at(&self) -> Instant {
        self.at
    }

    /// Tandai bahwa scene selesai dibangun dan penyerahan ke backend dimulai.
    ///
    /// Inilah yang memisahkan kerja kita dari *backpressure* swapchain. Bila
    /// tidak pernah dipanggil, seluruh durasi frame dihitung sebagai build.
    pub fn mark_built(&mut self, now: Instant) {
        self.built_at.get_or_insert(now);
    }

    /// Saat scene selesai dibangun, bila sudah ditandai.
    pub fn built_at(&self) -> Option<Instant> {
        self.built_at
    }
}

/// Scheduler render-on-dirty.
///
/// Kontraknya sederhana dan itulah intinya: selama tidak ada yang menandai
/// dirty, [`FrameScheduler::is_idle`] benar dan platform tidak boleh
/// menggambar apa pun — tidak ada loop yang berputar, tidak ada timer yang
/// berdetak. Begitu ada yang dirty, tepat **satu** frame dijadwalkan pada
/// vsync berikutnya, berapa pun laju layarnya.
#[derive(Debug)]
pub struct FrameScheduler {
    dirty: Dirty,
    awaiting: bool,
    visible: bool,
    frame: u64,
    vsync: Vsync,
    estimator: RefreshEstimator,
    stats: FrameStats,
    last_frame_at: Option<Instant>,
}

impl FrameScheduler {
    /// Scheduler baru: idle, terlihat, dan belum tahu detak layar.
    pub fn new() -> Self {
        Self {
            dirty: Dirty::NONE,
            awaiting: false,
            visible: true,
            frame: 0,
            vsync: Vsync::UNKNOWN,
            estimator: RefreshEstimator::new(),
            stats: FrameStats::new(),
            last_frame_at: None,
        }
    }

    /// Laporkan detak layar dari platform (CADisplayLink dsb.).
    ///
    /// Sumber yang lebih tepercaya menang: taksiran tidak akan menimpa angka
    /// yang datang langsung dari display link.
    pub fn set_vsync(&mut self, vsync: Vsync) {
        self.vsync = self.vsync.preferred(vsync);
    }

    /// Detak layar yang dipakai saat ini.
    pub fn vsync(&self) -> Vsync {
        self.vsync
    }

    /// Buang taksiran refresh rate (mis. window pindah monitor).
    pub fn reset_vsync_estimate(&mut self) {
        self.estimator.reset();
        if self.vsync.source() != ClockSource::DisplayLink {
            self.vsync = Vsync::UNKNOWN;
        }
    }

    /// Apakah window sedang terlihat.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Setel visibilitas window (occluded/minimize).
    ///
    /// Saat tersembunyi, permintaan frame tetap **dicatat** tapi tidak pernah
    /// membangunkan GPU; begitu terlihat lagi, utang itu langsung dibayar.
    pub fn set_visible(&mut self, visible: bool) -> Wake {
        if self.visible == visible {
            return if visible && self.awaiting {
                Wake::AlreadyScheduled
            } else {
                Wake::Suppressed
            };
        }
        self.visible = visible;
        if !visible {
            self.awaiting = false;
            return Wake::Suppressed;
        }
        if self.dirty.is_empty() {
            Wake::Suppressed
        } else {
            self.awaiting = true;
            Wake::Schedule
        }
    }

    /// Minta satu frame karena `dirty`.
    pub fn request(&mut self, dirty: Dirty) -> Wake {
        if dirty.is_empty() {
            return Wake::Suppressed;
        }
        self.dirty.insert(dirty);
        if !self.visible {
            return Wake::Suppressed;
        }
        if self.awaiting {
            return Wake::AlreadyScheduled;
        }
        self.awaiting = true;
        Wake::Schedule
    }

    /// Alasan-alasan yang belum dilayani.
    pub fn pending(&self) -> Dirty {
        self.dirty
    }

    /// Benar bila tidak ada apa pun yang perlu digambar.
    ///
    /// Inilah sinyal untuk platform mem-pause display link dan tidur di
    /// `ControlFlow::Wait`.
    pub fn is_idle(&self) -> bool {
        self.dirty.is_empty() && !self.awaiting
    }

    /// Benar bila sebuah frame sudah dijadwalkan dan sedang ditunggu.
    pub fn awaiting_frame(&self) -> bool {
        self.awaiting
    }

    /// Mulai satu frame; mengosongkan dirty dan memulai pengukuran.
    ///
    /// Boleh dipanggil walau `dirty` kosong: OS bisa meminta redraw sendiri
    /// (expose/occlusion). `reason` pada [`FrameStart`] akan kosong dan itu
    /// terbaca jelas di log.
    pub fn begin_frame(&mut self, now: Instant) -> FrameStart {
        let reason = self.dirty;
        self.dirty.clear();
        self.awaiting = false;
        FrameStart {
            index: self.frame,
            reason,
            at: now,
            built_at: None,
        }
    }

    /// Tutup frame, catat statistik, dan perbarui taksiran refresh rate.
    pub fn end_frame(&mut self, start: FrameStart, now: Instant, presented: bool) -> FrameTiming {
        let built_at = start.built_at.unwrap_or(now).clamp(start.at, now);
        let build = built_at.saturating_duration_since(start.at);
        let present = now.saturating_duration_since(built_at);
        let since_previous = self
            .last_frame_at
            .map(|t| start.at.saturating_duration_since(t));

        if presented {
            if let Some(delta) = since_previous {
                if self.estimator.observe(delta) && self.vsync.source() != ClockSource::DisplayLink
                {
                    if let Some(est) = self.estimator.estimate().and_then(Vsync::estimated) {
                        self.vsync = self.vsync.preferred(est);
                    }
                }
            }
            self.last_frame_at = Some(start.at);
            self.frame += 1;
        }

        // Dinilai dari `build` saja: `present` didominasi antrean swapchain
        // yang memang menahan sampai vsync berikutnya.
        let over_budget = self.vsync.budget().is_some_and(|b| build > b);
        let timing = FrameTiming {
            index: start.index,
            reason: start.reason,
            build,
            present,
            since_previous,
            presented,
            over_budget,
        };
        self.stats.record(timing);
        timing
    }

    /// Nomor frame berikutnya yang akan digambar.
    pub fn frame_index(&self) -> u64 {
        self.frame
    }

    /// Statistik frame time.
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn dirty_adalah_bitset_yang_terbaca() {
        let d = Dirty::LAYOUT | Dirty::ANIMATION;
        assert!(d.contains(Dirty::LAYOUT));
        assert!(d.contains(Dirty::ANIMATION));
        assert!(!d.contains(Dirty::THEME));
        assert!(!d.is_empty());
        assert!(Dirty::NONE.is_empty());
        assert_eq!(d.to_string(), "layout+animation");
        assert_eq!(Dirty::NONE.to_string(), "none");
        assert_eq!(format!("{:?}", Dirty::PAINT), "Dirty(paint)");
    }

    #[test]
    fn idle_berarti_tidak_menggambar_sama_sekali() {
        let mut s = FrameScheduler::new();
        assert!(s.is_idle());
        assert_eq!(s.request(Dirty::NONE), Wake::Suppressed);
        assert!(s.is_idle(), "Dirty::NONE tidak boleh membangunkan renderer");
    }

    #[test]
    fn satu_frame_untuk_banyak_permintaan() {
        let mut s = FrameScheduler::new();
        assert_eq!(s.request(Dirty::PAINT), Wake::Schedule);
        assert_eq!(s.request(Dirty::LAYOUT), Wake::AlreadyScheduled);
        assert_eq!(s.request(Dirty::THEME), Wake::AlreadyScheduled);
        assert!(!s.is_idle());

        let start = s.begin_frame(t0());
        assert!(start
            .reason()
            .contains(Dirty::PAINT | Dirty::LAYOUT | Dirty::THEME));
        assert!(s.is_idle(), "dirty harus bersih begitu frame dimulai");
    }

    #[test]
    fn animasi_menjadwalkan_frame_berikutnya_dari_dalam_frame() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        // Scene fn menyatakan spring-nya belum selesai.
        assert_eq!(s.request(Dirty::ANIMATION), Wake::Schedule);
        s.end_frame(start, a + Duration::from_millis(1), true);
        assert!(!s.is_idle());
        assert!(s.pending().contains(Dirty::ANIMATION));
    }

    #[test]
    fn window_tersembunyi_tidak_pernah_membangunkan_gpu() {
        let mut s = FrameScheduler::new();
        assert_eq!(s.set_visible(false), Wake::Suppressed);
        assert_eq!(s.request(Dirty::PAINT), Wake::Suppressed);
        assert!(!s.awaiting_frame());
        // Utangnya tetap tercatat dan dibayar begitu terlihat lagi.
        assert!(s.pending().contains(Dirty::PAINT));
        assert_eq!(s.set_visible(true), Wake::Schedule);
        assert!(s.awaiting_frame());
    }

    #[test]
    fn terlihat_lagi_tanpa_utang_tidak_menggambar() {
        let mut s = FrameScheduler::new();
        s.set_visible(false);
        assert_eq!(s.set_visible(true), Wake::Suppressed);
        assert!(s.is_idle());
    }

    #[test]
    fn frame_yang_diskip_tidak_menaikkan_nomor_frame() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        let timing = s.end_frame(start, a + Duration::from_millis(3), false);
        assert!(!timing.presented);
        assert_eq!(s.frame_index(), 0);
        assert_eq!(s.stats().skipped(), 1);
        assert_eq!(s.stats().presented(), 0);
    }

    #[test]
    fn waktu_build_dan_present_dipisah() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let mut start = s.begin_frame(a);
        start.mark_built(a + Duration::from_micros(2_500));
        let timing = s.end_frame(start, a + Duration::from_millis(10), true);
        assert_eq!(timing.build, Duration::from_micros(2_500));
        assert_eq!(timing.present, Duration::from_micros(7_500));
        assert_eq!(timing.total(), Duration::from_millis(10));
        assert_eq!(timing.since_previous, None);
        assert_eq!(s.frame_index(), 1);
    }

    #[test]
    fn tanpa_mark_built_semua_dihitung_sebagai_build() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        let timing = s.end_frame(start, a + Duration::from_millis(4), true);
        assert_eq!(timing.build, Duration::from_millis(4));
        assert_eq!(timing.present, Duration::ZERO);
    }

    #[test]
    fn backpressure_swapchain_tidak_dituduh_frame_lambat() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.request(Dirty::ANIMATION);
        let a = t0();
        let mut start = s.begin_frame(a);
        // Build 1 ms (sehat), lalu present memblok 12 ms menunggu buffer bebas.
        start.mark_built(a + Duration::from_millis(1));
        let timing = s.end_frame(start, a + Duration::from_millis(13), true);
        assert!(!timing.over_budget, "menunggu vsync bukan frame lambat");
        assert!(timing.total() > Duration::from_millis(12));
    }

    #[test]
    fn vsync_tidak_pernah_menebak_saat_belum_tahu() {
        let s = FrameScheduler::new();
        assert_eq!(s.vsync(), Vsync::UNKNOWN);
        assert_eq!(s.vsync().budget(), None);
        assert_eq!(s.vsync().hz(), None);
        assert!(!s.vsync().is_known());
    }

    #[test]
    fn vsync_menolak_interval_tak_masuk_akal() {
        assert!(Vsync::display_link(Duration::from_secs(1)).is_none());
        assert!(Vsync::display_link(Duration::from_nanos(10)).is_none());
        assert!(Vsync::from_hz(0.0, ClockSource::DisplayLink).is_none());
        assert!(Vsync::from_hz(f64::NAN, ClockSource::DisplayLink).is_none());
        assert!(Vsync::from_hz(120.0, ClockSource::DisplayLink).is_some());
    }

    #[test]
    fn budget_mengikuti_promotion_bukan_konstanta() {
        let v60 = Vsync::from_hz(60.0, ClockSource::DisplayLink).unwrap();
        let v120 = Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap();
        assert!(v120.budget().unwrap() < v60.budget().unwrap());
        // 120 Hz ≈ 8,33 ms — separuh dari 60 Hz, bukan 16,6 ms.
        let b = v120.budget().unwrap().as_secs_f64() * 1000.0;
        assert!((b - 8.333).abs() < 0.01, "budget 120 Hz = {b} ms");
    }

    #[test]
    fn display_link_mengalahkan_taksiran() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.set_vsync(Vsync::from_hz(60.0, ClockSource::Estimated).unwrap());
        assert_eq!(s.vsync().source(), ClockSource::DisplayLink);
        assert!((s.vsync().hz().unwrap() - 120.0).abs() < 0.001);
    }

    #[test]
    fn display_link_baru_menimpa_display_link_lama() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(60.0, ClockSource::DisplayLink).unwrap());
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        assert!((s.vsync().hz().unwrap() - 120.0).abs() < 0.001);
    }

    #[test]
    fn taksiran_butuh_cukup_sampel_dan_menolak_jeda_idle() {
        let mut e = RefreshEstimator::new();
        assert_eq!(e.estimate(), None);
        assert!(
            !e.observe(Duration::from_secs(3)),
            "jeda idle harus ditolak"
        );
        for _ in 0..ESTIMATOR_MIN_SAMPLES - 1 {
            assert!(e.observe(Duration::from_micros(8_333)));
        }
        assert_eq!(e.estimate(), None, "belum cukup sampel");
        assert!(e.observe(Duration::from_micros(8_333)));
        assert_eq!(e.estimate(), Some(Duration::from_micros(8_333)));
    }

    #[test]
    fn taksiran_memakai_median_sehingga_tahan_frame_drop() {
        let mut e = RefreshEstimator::new();
        for i in 0..16 {
            // Satu dari empat frame di-drop (dua kali interval).
            let d = if i % 4 == 3 {
                Duration::from_micros(16_666)
            } else {
                Duration::from_micros(8_333)
            };
            e.observe(d);
        }
        assert_eq!(e.estimate(), Some(Duration::from_micros(8_333)));
    }

    #[test]
    fn scheduler_menaksir_vsync_di_platform_tanpa_display_link() {
        let mut s = FrameScheduler::new();
        let mut now = t0();
        for _ in 0..12 {
            s.request(Dirty::ANIMATION);
            let start = s.begin_frame(now);
            s.end_frame(start, now + Duration::from_micros(900), true);
            now += Duration::from_micros(8_333);
        }
        let v = s.vsync();
        assert_eq!(v.source(), ClockSource::Estimated);
        assert!((v.hz().unwrap() - 120.0).abs() < 1.0, "hz = {:?}", v.hz());
    }

    #[test]
    fn jeda_idle_panjang_tidak_meracuni_taksiran() {
        let mut s = FrameScheduler::new();
        let mut now = t0();
        for i in 0..14 {
            s.request(Dirty::ANIMATION);
            let start = s.begin_frame(now);
            s.end_frame(start, now + Duration::from_micros(500), true);
            // Sesekali aplikasi benar-benar idle selama 5 detik.
            now += if i % 5 == 4 {
                Duration::from_secs(5)
            } else {
                Duration::from_micros(8_333)
            };
        }
        assert!((s.vsync().hz().unwrap() - 120.0).abs() < 1.0);
    }

    #[test]
    fn over_budget_hanya_dinilai_saat_vsync_diketahui() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        let t = s.end_frame(start, a + Duration::from_millis(50), true);
        assert!(
            !t.over_budget,
            "tanpa tahu vsync, jangan menuduh frame lambat"
        );
        assert_eq!(t.build, Duration::from_millis(50));

        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.request(Dirty::PAINT);
        let b = a + Duration::from_millis(100);
        let start = s.begin_frame(b);
        let t = s.end_frame(start, b + Duration::from_millis(50), true);
        assert!(t.over_budget);
        assert_eq!(s.stats().over_budget(), 1);
    }

    #[test]
    fn statistik_persentil_dan_worst() {
        let mut stats = FrameStats::new();
        for i in 1..=10u64 {
            stats.record(FrameTiming {
                index: i,
                reason: Dirty::PAINT,
                build: Duration::from_millis(i),
                present: Duration::ZERO,
                since_previous: None,
                presented: true,
                over_budget: false,
            });
        }
        assert_eq!(stats.frames(), 10);
        assert_eq!(stats.worst(), Duration::from_millis(10));
        assert_eq!(stats.p50(), Some(Duration::from_millis(5)));
        assert_eq!(stats.p95(), Some(Duration::from_millis(10)));
        assert_eq!(stats.average(), Some(Duration::from_micros(5_500)));
        assert_eq!(stats.percentile(1.1), None);
    }

    #[test]
    fn logger_mencatat_frame_lambat_dan_ringkasan_berkala() {
        let logger = FrameLogger::every(4);
        let mut stats = FrameStats::new();
        let vsync = Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap();

        let cepat = FrameTiming {
            index: 1,
            reason: Dirty::PAINT,
            build: Duration::from_micros(900),
            present: Duration::from_micros(7_000),
            since_previous: Some(Duration::from_micros(8_333)),
            presented: true,
            over_budget: false,
        };
        stats.record(cepat);
        assert_eq!(logger.line(&stats, vsync, &cepat), None);

        let lambat = FrameTiming {
            index: 2,
            over_budget: true,
            build: Duration::from_millis(20),
            ..cepat
        };
        stats.record(lambat);
        let line = logger.line(&stats, vsync, &lambat).expect("harus dicatat");
        assert!(line.contains("LAMBAT"), "{line}");
        assert!(line.contains("120.0 Hz (display-link)"), "{line}");
        assert!(line.contains("paint"), "{line}");

        let berkala = FrameTiming { index: 4, ..cepat };
        stats.record(berkala);
        assert!(logger.line(&stats, vsync, &berkala).is_some());
    }

    #[test]
    fn logger_tanpa_vsync_tidak_mengarang_angka() {
        let logger = FrameLogger::every(1);
        let mut stats = FrameStats::new();
        let t = FrameTiming {
            index: 1,
            reason: Dirty::EXTERNAL,
            build: Duration::from_millis(9),
            present: Duration::ZERO,
            since_previous: None,
            presented: true,
            over_budget: false,
        };
        stats.record(t);
        let line = logger.line(&stats, Vsync::UNKNOWN, &t).unwrap();
        assert!(line.contains("? Hz (unknown)"), "{line}");
        assert!(line.contains("Δ ?"), "{line}");
        assert!(!line.contains("16.6"), "tidak boleh ada konstanta 16,6 ms");
    }

    #[test]
    fn ringkasan_stats_menyebut_budget_yang_tidak_diketahui() {
        let stats = FrameStats::new();
        let s = stats.summary(Vsync::UNKNOWN);
        assert!(s.contains("budget ?"), "{s}");
    }

    #[test]
    fn reset_taksiran_tidak_menghapus_angka_display_link() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.reset_vsync_estimate();
        assert_eq!(s.vsync().source(), ClockSource::DisplayLink);

        let mut s2 = FrameScheduler::new();
        s2.set_vsync(Vsync::from_hz(60.0, ClockSource::Estimated).unwrap());
        s2.reset_vsync_estimate();
        assert_eq!(s2.vsync(), Vsync::UNKNOWN);
    }
}
