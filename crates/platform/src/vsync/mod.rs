//! Sumber detak vsync per platform (REKOMENDASI §3.5).
//!
//! Scheduler di `rustui-core` memutuskan **apakah** perlu menggambar; modul ini
//! memutuskan **kapan** — dan menjawabnya dengan bertanya ke OS, bukan dengan
//! menebak.
//!
//! | Platform | Sumber detak | Interval |
//! |---|---|---|
//! | macOS | `CADisplayLink` di run loop utama | `targetTimestamp - timestamp` tiap tick — ikut ProMotion 120 Hz, adaptive refresh, dan perpindahan monitor |
//! | lain | [`winit::window::Window::request_redraw`] | ditaksir dari jarak antar-frame nyata oleh [`rustui_core::scheduler::RefreshEstimator`] |
//!
//! **Tidak ada 16,6 ms di mana pun.** Kalau interval belum diketahui, ia
//! bernilai `None` dan lapisan di atas menanganinya sebagai ketidaktahuan.
//!
//! ## Idle tetap idle
//!
//! Display link adalah timer yang berdetak terus — persis yang dilarang §3.5.
//! Karena itu ia dibuat dalam keadaan **paused**: [`VsyncSource::schedule`]
//! melepasnya hanya ketika ada yang dirty, dan [`VsyncSource::idle`]
//! menghentikannya lagi begitu frame selesai tanpa sisa pekerjaan. Saat
//! aplikasi diam, tidak ada satu pun callback yang berjalan.

#[cfg(target_os = "macos")]
mod macos;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rustui_core::scheduler::Vsync;
use winit::window::Window;

/// Dari mana detak frame datang di proses ini.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VsyncKind {
    /// `CADisplayLink` macOS — mengikuti laju layar yang sedang aktif.
    DisplayLink,
    /// `request_redraw` winit; interval ditaksir dari frame yang benar-benar
    /// terjadi.
    RequestRedraw,
}

impl VsyncKind {
    /// Nama pendek untuk log.
    pub const fn label(self) -> &'static str {
        match self {
            VsyncKind::DisplayLink => "CADisplayLink",
            VsyncKind::RequestRedraw => "request_redraw",
        }
    }
}

/// Jam vsync yang dibagi antara callback OS dan event loop.
///
/// Callback display link berjalan di run loop utama, sama seperti event loop
/// winit, tapi keduanya adalah *reentrancy boundary* yang berbeda — jadi
/// nilainya disimpan sebagai atomik alih-alih dipinjam.
#[derive(Debug, Default)]
pub struct VsyncClock {
    /// Interval terakhir yang dilaporkan OS, dalam nanodetik. `0` = belum tahu.
    interval_nanos: AtomicU64,
    ticks: AtomicU64,
}

impl VsyncClock {
    /// Jam kosong: belum ada tick, interval belum diketahui.
    pub fn new() -> Self {
        Self::default()
    }

    /// Catat satu tick beserta interval yang dilaporkan OS.
    pub fn tick(&self, interval: Option<Duration>) {
        if let Some(d) = interval {
            let nanos = d.as_nanos().min(u64::MAX as u128) as u64;
            if nanos > 0 {
                self.interval_nanos.store(nanos, Ordering::Relaxed);
            }
        }
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// Setel interval tanpa menghitungnya sebagai tick (mis. nilai awal dari
    /// `NSScreen.maximumFramesPerSecond`).
    pub fn seed_interval(&self, interval: Duration) {
        let nanos = interval.as_nanos().min(u64::MAX as u128) as u64;
        if nanos > 0 {
            self.interval_nanos
                .compare_exchange(0, nanos, Ordering::Relaxed, Ordering::Relaxed)
                .ok();
        }
    }

    /// Interval vsync terakhir yang dilaporkan OS.
    pub fn interval(&self) -> Option<Duration> {
        match self.interval_nanos.load(Ordering::Relaxed) {
            0 => None,
            n => Some(Duration::from_nanos(n)),
        }
    }

    /// Jumlah tick sejak jam dibuat.
    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    /// Detak layar dalam bentuk yang dimengerti scheduler.
    pub fn vsync(&self) -> Vsync {
        self.interval()
            .and_then(Vsync::display_link)
            .unwrap_or(Vsync::UNKNOWN)
    }
}

/// Sumber detak vsync untuk satu window.
///
/// Bentuknya sengaja hanya dua tombol — [`VsyncSource::schedule`] dan
/// [`VsyncSource::idle`] — supaya jalur macOS dan jalur fallback benar-benar
/// dipakai lewat kode yang sama di event loop.
pub struct VsyncSource {
    window: Arc<Window>,
    clock: Arc<VsyncClock>,
    kind: VsyncKind,
    #[cfg(target_os = "macos")]
    link: Option<macos::DisplayLink>,
}

impl VsyncSource {
    /// Pasang sumber vsync terbaik yang tersedia untuk `window`.
    ///
    /// Di macOS ini mencoba `CADisplayLink` (butuh macOS 14+); bila tidak
    /// tersedia — dan di seluruh OS lain — ia turun ke `request_redraw` winit
    /// tanpa mengubah kontrak apa pun bagi pemanggil.
    pub fn attach(window: Arc<Window>) -> Self {
        let clock = Arc::new(VsyncClock::new());

        #[cfg(target_os = "macos")]
        {
            let notify = {
                let window = window.clone();
                move || window.request_redraw()
            };
            if let Some(link) = macos::DisplayLink::attach(&window, clock.clone(), notify) {
                return Self {
                    window,
                    clock,
                    kind: VsyncKind::DisplayLink,
                    link: Some(link),
                };
            }
        }

        Self {
            window,
            clock,
            kind: VsyncKind::RequestRedraw,
            #[cfg(target_os = "macos")]
            link: None,
        }
    }

    /// Sumber detak yang benar-benar dipakai.
    pub fn kind(&self) -> VsyncKind {
        self.kind
    }

    /// Jam vsync bersama — dibaca event loop tiap frame.
    pub fn clock(&self) -> &Arc<VsyncClock> {
        &self.clock
    }

    /// Detak layar yang dilaporkan OS, bila sudah diketahui.
    pub fn vsync(&self) -> Vsync {
        self.clock.vsync()
    }

    /// Minta satu frame pada vsync berikutnya.
    pub fn schedule(&self) {
        #[cfg(target_os = "macos")]
        if let Some(link) = self.link.as_ref() {
            link.set_paused(false);
            return;
        }
        self.window.request_redraw();
    }

    /// Tidak ada lagi yang perlu digambar — hentikan detak sampai dibangunkan.
    pub fn idle(&self) {
        #[cfg(target_os = "macos")]
        if let Some(link) = self.link.as_ref() {
            link.set_paused(true);
        }
    }
}

impl core::fmt::Debug for VsyncSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VsyncSource")
            .field("kind", &self.kind)
            .field("interval", &self.clock.interval())
            .field("ticks", &self.clock.ticks())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustui_core::scheduler::ClockSource;

    #[test]
    fn jam_kosong_tidak_mengarang_interval() {
        let c = VsyncClock::new();
        assert_eq!(c.interval(), None);
        assert_eq!(c.ticks(), 0);
        assert_eq!(c.vsync(), Vsync::UNKNOWN);
        assert!(!c.vsync().is_known());
    }

    #[test]
    fn tick_membawa_interval_promotion() {
        let c = VsyncClock::new();
        // 120 Hz: 8,333 ms — bukan 16,6 ms.
        c.tick(Some(Duration::from_nanos(8_333_333)));
        assert_eq!(c.ticks(), 1);
        let v = c.vsync();
        assert_eq!(v.source(), ClockSource::DisplayLink);
        assert!((v.hz().unwrap() - 120.0).abs() < 0.1);
    }

    #[test]
    fn tick_tanpa_interval_tetap_dihitung() {
        let c = VsyncClock::new();
        c.tick(None);
        c.tick(None);
        assert_eq!(c.ticks(), 2);
        assert_eq!(c.interval(), None);
    }

    #[test]
    fn interval_terbaru_menang_saat_laju_layar_berubah() {
        let c = VsyncClock::new();
        c.tick(Some(Duration::from_nanos(16_666_667))); // 60 Hz
        c.tick(Some(Duration::from_nanos(8_333_333))); // naik ke 120 Hz
        assert!((c.vsync().hz().unwrap() - 120.0).abs() < 0.1);
    }

    #[test]
    fn seed_hanya_mengisi_saat_masih_kosong() {
        let c = VsyncClock::new();
        c.seed_interval(Duration::from_nanos(16_666_667));
        assert!((c.vsync().hz().unwrap() - 60.0).abs() < 0.1);
        // Tick sungguhan dari display link berhak menimpa seed.
        c.tick(Some(Duration::from_nanos(8_333_333)));
        assert!((c.vsync().hz().unwrap() - 120.0).abs() < 0.1);
        // Seed berikutnya tidak boleh menurunkan lagi.
        c.seed_interval(Duration::from_nanos(16_666_667));
        assert!((c.vsync().hz().unwrap() - 120.0).abs() < 0.1);
        assert_eq!(c.ticks(), 1, "seed bukan tick");
    }

    #[test]
    fn interval_nol_diabaikan() {
        let c = VsyncClock::new();
        c.tick(Some(Duration::ZERO));
        c.seed_interval(Duration::ZERO);
        assert_eq!(c.interval(), None);
    }

    #[test]
    fn label_sumber_terbaca_di_log() {
        assert_eq!(VsyncKind::DisplayLink.label(), "CADisplayLink");
        assert_eq!(VsyncKind::RequestRedraw.label(), "request_redraw");
    }
}
