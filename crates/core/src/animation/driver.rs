//! Sambungan animasi ke scheduler: siapa yang meminta frame berikutnya.

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::scheduler::Dirty;

use super::motion::Motion;
use super::value::{Animatable, SpringValue};

/// Penggerak animasi satu window.
///
/// Tugasnya cuma dua, dan keduanya penting justru karena kecil:
///
/// 1. **Menghitung `dt` yang jujur** dari waktu frame yang diberikan platform —
///    tidak pernah dari konstanta 16,6 ms (§3.5).
/// 2. **Menjawab pertanyaan scheduler**: apakah masih ada yang bergerak? Kalau
///    tidak ada satu pun spring yang melapor aktif, [`AnimationDriver::end_frame`]
///    mengembalikan [`Dirty::NONE`] dan renderer benar-benar tidur.
///
/// ```
/// use std::time::{Duration, Instant};
/// use silka_core::animation::{AnimationDriver, SpringValue};
/// use silka_core::scheduler::{Dirty, FrameScheduler};
///
/// let mut scheduler = FrameScheduler::new();
/// let mut driver = AnimationDriver::new();
/// let mut x = SpringValue::new(0.0);
///
/// x.set_target(1.0);
/// scheduler.request(Dirty::ANIMATION);
///
/// let mut now = Instant::now();
/// while !scheduler.is_idle() {
///     let start = scheduler.begin_frame(now);
///     let tick = driver.begin_frame(now);
///     let _posisi = tick.advance(&mut x); // dipakai layout/paint frame ini
///     let lagi = driver.end_frame(tick);  // ANIMATION atau NONE
///     scheduler.request(lagi);
///     scheduler.end_frame(start, now, true);
///     now += Duration::from_micros(8_333);
/// }
/// assert_eq!(x.position(), 1.0);
/// ```
#[derive(Debug)]
pub struct AnimationDriver {
    motion: Motion,
    last: Option<Instant>,
    animating: bool,
}

impl AnimationDriver {
    /// Penggerak baru: belum punya jam, belum ada yang bergerak.
    pub fn new() -> Self {
        Self {
            motion: Motion::Full,
            last: None,
            animating: false,
        }
    }

    /// Preferensi gerakan yang berlaku.
    pub fn motion(&self) -> Motion {
        self.motion
    }

    /// Laporkan setting reduced-motion dari OS.
    ///
    /// Mengembalikan [`Dirty::ANIMATION`] bila nilainya berubah: gerakan
    /// dekoratif yang sedang berjalan perlu satu frame untuk menyelesaikan
    /// dirinya, dan tanpa permintaan itu ia akan membeku di tengah jalan.
    pub fn set_motion(&mut self, motion: Motion) -> Dirty {
        if self.motion == motion {
            return Dirty::NONE;
        }
        self.motion = motion;
        Dirty::ANIMATION
    }

    /// Benar bila frame sebelumnya masih ada yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Buang jam (window pindah monitor, aplikasi baru bangun dari suspend).
    ///
    /// Frame berikutnya akan ber-`dt` nol, bukan selisih raksasa.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Mulai satu frame animasi pada `now`.
    ///
    /// `dt` adalah jarak ke frame animasi sebelumnya. Setelah periode idle
    /// jam sengaja dilupakan ([`AnimationDriver::end_frame`]), sehingga frame
    /// pertama sebuah animasi selalu `dt = 0` — gerakan dimulai dari keadaan
    /// yang benar-benar terlihat pengguna, bukan meloncat sejauh lamanya
    /// aplikasi diam.
    pub fn begin_frame(&mut self, now: Instant) -> Tick {
        let dt = match self.last {
            Some(prev) => now.saturating_duration_since(prev),
            None => Duration::ZERO,
        };
        self.last = Some(now);
        Tick {
            dt,
            motion: self.motion,
            active: Cell::new(false),
        }
    }

    /// Tutup frame; mengembalikan alasan dirty untuk frame berikutnya.
    ///
    /// [`Dirty::ANIMATION`] bila masih ada yang bergerak, [`Dirty::NONE`] bila
    /// semuanya sudah berhenti — dan begitu berhenti, jamnya dilupakan.
    pub fn end_frame(&mut self, tick: Tick) -> Dirty {
        self.animating = tick.active.get();
        if self.animating {
            Dirty::ANIMATION
        } else {
            self.last = None;
            Dirty::NONE
        }
    }
}

impl Default for AnimationDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Token satu frame animasi.
///
/// Dibagikan ke seluruh pohon selama frame berlangsung. Setiap nilai yang
/// masih bergerak menandai dirinya di sini lewat [`Tick::advance`], dan
/// penandaan itulah yang membuat frame berikutnya dijadwalkan — bukan sebuah
/// timer yang berdetak terus-menerus.
///
/// Penandaannya memakai [`Cell`] supaya `&Tick` cukup: kode paint memegangnya
/// sebagai referensi bersama, tanpa perlu `&mut` yang akan menular ke seluruh
/// tanda tangan fungsi widget.
#[derive(Debug)]
pub struct Tick {
    dt: Duration,
    motion: Motion,
    active: Cell<bool>,
}

impl Tick {
    /// Tick manual — untuk uji dan untuk pemanggil yang mengurus jamnya sendiri.
    pub fn manual(dt: Duration, motion: Motion) -> Self {
        Self {
            dt,
            motion,
            active: Cell::new(false),
        }
    }

    /// Jarak waktu ke frame animasi sebelumnya.
    pub fn dt(&self) -> Duration {
        self.dt
    }

    /// Preferensi gerakan yang berlaku frame ini.
    pub fn motion(&self) -> Motion {
        self.motion
    }

    /// Majukan sebuah nilai dan kembalikan posisinya untuk frame ini.
    ///
    /// Nilai yang masih bergerak otomatis meminta frame berikutnya.
    pub fn advance<T: Animatable>(&self, value: &mut SpringValue<T>) -> T {
        if value.advance(self.dt, self.motion) {
            self.active.set(true);
        }
        value.position()
    }

    /// Minta frame berikutnya tanpa lewat [`SpringValue`].
    ///
    /// Untuk sumber gerakan lain (video, indikator progres tak tentu) yang
    /// tetap harus tunduk pada aturan yang sama.
    pub fn keep_awake(&self) {
        self.active.set(true);
    }

    /// Benar bila ada yang menandai dirinya masih bergerak.
    pub fn is_active(&self) -> bool {
        self.active.get()
    }
}
