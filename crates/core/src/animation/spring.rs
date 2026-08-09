//! Parameter spring dan solusi **closed-form damped harmonic oscillator**.
//!
//! Parameternya *perceptual* — durasi + bounce, persis WWDC23 "Animate with
//! springs" — bukan mass/stiffness/damping. Yang terakhir tetap bisa dipakai
//! ([`Spring::physical`]) tapi bukan bahasa utamanya: seorang desainer bisa
//! menjawab "berapa lama dan seberapa memantul", tidak "berapa newton per
//! meter".

use core::f32::consts::TAU;

use super::value::Tolerance;
use super::Animatable;

/// Durasi perceptual terpendek yang diterima (1 ms).
///
/// Nol akan membuat frekuensi tak hingga; membatasi di sini membuat
/// `Spring::new(0.0, _)` tetap menghasilkan spring yang sah (sangat cepat)
/// alih-alih NaN yang menjalar ke seluruh pohon.
pub const MIN_DURATION: f32 = 0.001;

/// Batas |bounce| yang diterima.
///
/// `bounce = 1` berarti damping nol (berayun selamanya, tidak pernah settle);
/// `bounce = -1` berarti damping tak hingga. Keduanya bukan animasi UI.
pub const MAX_BOUNCE: f32 = 0.99;

/// Selisih rasio damping terhadap 1.0 yang masih dianggap *critically damped*.
///
/// Di sekitar ζ = 1 bentuk underdamped dan overdamped sama-sama membagi dengan
/// angka yang mendekati nol; cabang kritis adalah limit analitik keduanya, jadi
/// memakainya di pita sempit ini bukan aproksimasi kasar melainkan cara
/// menghindari pembagian tak stabil.
const CRITICAL_BAND: f32 = 1.0e-4;

/// Sebuah spring: durasi perceptual + bounce (WWDC23), disimpan bersama bentuk
/// fisiknya (frekuensi sudut ω dan rasio damping ζ).
///
/// ```
/// use silka_core::animation::Spring;
///
/// let s = Spring::snappy();
/// assert!((s.duration() - 0.5).abs() < 1e-6);
/// assert!((s.damping_ratio() - 0.85).abs() < 1e-6);
/// // Tidak memantul kalau bounce-nya dibuang (dipakai saat reduced-motion).
/// assert!(s.without_bounce().damping_ratio() >= 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    duration: f32,
    bounce: f32,
    omega: f32,
    zeta: f32,
}

impl Spring {
    /// Spring dari **durasi perceptual** (detik) dan **bounce**.
    ///
    /// - `bounce == 0` — critically damped: sampai secepat mungkin tanpa
    ///   melewati target sama sekali.
    /// - `bounce > 0` — underdamped: melewati target lalu kembali (rasa
    ///   "hidup"). ζ = 1 − bounce.
    /// - `bounce < 0` — overdamped: merayap masuk, tidak pernah melewati.
    ///   ζ = 1 / (1 + bounce).
    ///
    /// Angka di luar rentang wajar dijepit ([`MIN_DURATION`], [`MAX_BOUNCE`])
    /// alih-alih memanik: satu literal salah di kode widget tidak boleh
    /// menjatuhkan aplikasi.
    pub fn new(duration: f32, bounce: f32) -> Self {
        let duration = if duration.is_finite() {
            duration.max(MIN_DURATION)
        } else {
            MIN_DURATION
        };
        let bounce = if bounce.is_finite() {
            bounce.clamp(-MAX_BOUNCE, MAX_BOUNCE)
        } else {
            0.0
        };
        let omega = TAU / duration;
        let zeta = if bounce >= 0.0 {
            1.0 - bounce
        } else {
            1.0 / (1.0 + bounce)
        };
        Self {
            duration,
            bounce,
            omega,
            zeta,
        }
    }

    /// Preset `smooth` ala SwiftUI: tanpa pantulan sama sekali.
    ///
    /// Default framework — dipakai untuk hover, fokus, dan perubahan warna:
    /// segala sesuatu yang tidak boleh menarik perhatian ke dirinya sendiri.
    pub fn smooth() -> Self {
        Self::new(0.5, 0.0)
    }

    /// Preset `snappy`: sedikit pantulan, terasa responsif.
    ///
    /// Untuk kontrol yang ditekan dan langsung menjawab — tombol, toggle,
    /// segmented control.
    pub fn snappy() -> Self {
        Self::new(0.5, 0.15)
    }

    /// Preset `bouncy`: pantulan jelas, terasa main-main.
    ///
    /// Untuk elemen besar yang muncul/menghilang — sheet, popover — di mana
    /// pantulan justru memperjelas arah gerakan.
    pub fn bouncy() -> Self {
        Self::new(0.5, 0.3)
    }

    /// Spring dari parameter fisik (massa, kekakuan, damping).
    ///
    /// Disediakan untuk memindahkan nilai dari sistem lain; bahasa utama tetap
    /// [`Spring::new`].
    pub fn physical(mass: f32, stiffness: f32, damping: f32) -> Self {
        let mass = if mass.is_finite() && mass > 0.0 {
            mass
        } else {
            1.0
        };
        let stiffness = if stiffness.is_finite() && stiffness > 0.0 {
            stiffness
        } else {
            1.0
        };
        let damping = if damping.is_finite() && damping >= 0.0 {
            damping
        } else {
            0.0
        };
        let omega = (stiffness / mass).sqrt();
        let zeta = damping / (2.0 * (stiffness * mass).sqrt());
        let duration = TAU / omega;
        let bounce = if zeta <= 1.0 {
            1.0 - zeta
        } else {
            1.0 / zeta - 1.0
        };
        Self::new(duration, bounce)
    }

    /// Durasi perceptual (detik).
    pub fn duration(self) -> f32 {
        self.duration
    }

    /// Bounce, dalam rentang −[`MAX_BOUNCE`]..=[`MAX_BOUNCE`].
    pub fn bounce(self) -> f32 {
        self.bounce
    }

    /// Rasio damping ζ. `1.0` = critically damped.
    pub fn damping_ratio(self) -> f32 {
        self.zeta
    }

    /// Frekuensi sudut ω (rad/detik).
    pub fn angular_frequency(self) -> f32 {
        self.omega
    }

    /// Kekakuan setara (massa = 1).
    pub fn stiffness(self) -> f32 {
        self.omega * self.omega
    }

    /// Koefisien damping setara (massa = 1).
    pub fn damping(self) -> f32 {
        2.0 * self.zeta * self.omega
    }

    /// Benar bila spring ini akan melewati target (bounce positif).
    pub fn overshoots(self) -> bool {
        self.zeta < 1.0 - CRITICAL_BAND
    }

    /// Spring yang sama tapi tanpa pantulan.
    ///
    /// Inilah yang dipakai [`super::Motion::Reduced`]: reduced-motion
    /// mematikan *bounce*, bukan mematikan seluruh gerakan
    /// (INTEGRASI-NATIVE §"Reduced motion"). Spring yang memang sudah
    /// overdamped dibiarkan apa adanya.
    pub fn without_bounce(self) -> Self {
        if self.bounce > 0.0 {
            Self::new(self.duration, 0.0)
        } else {
            self
        }
    }

    /// Salinan dengan durasi perceptual lain.
    pub fn with_duration(self, duration: f32) -> Self {
        Self::new(duration, self.bounce)
    }

    /// Salinan dengan bounce lain.
    pub fn with_bounce(self, bounce: f32) -> Self {
        Self::new(self.duration, bounce)
    }

    /// Matriks perambatan keadaan `(simpangan, kecepatan)` selama `t` detik.
    ///
    /// Inilah inti sistem animasi ini. Persamaan `x'' + 2ζω x' + ω² x = 0`
    /// **linear**, jadi keadaan setelah `t` selalu berupa kombinasi linear dari
    /// keadaan sekarang — koefisiennya hanya bergantung pada `t`, tidak pada
    /// nilai. Tiga konsekuensi yang membentuk seluruh API di atasnya:
    ///
    /// 1. **Tidak ada waktu-mulai yang perlu disimpan.** Setiap frame
    ///    diselesaikan dari keadaan *sekarang*, sehingga
    ///    [`super::SpringValue::set_target`] cukup mengganti target — velocity
    ///    ikut terbawa tanpa perlakuan khusus (WWDC23).
    /// 2. **Hasilnya tidak bergantung ukuran langkah.** Satu langkah 100 ms
    ///    sama dengan dua belas langkah 8,3 ms; frame yang di-drop tidak
    ///    menggeser animasi, dan tidak ada integrator yang bisa meledak.
    /// 3. **Satu matriks untuk semua komponen.** Point, Size, dan Color
    ///    memakai koefisien yang sama, jadi vektor tidak berarti kerja berlipat
    ///    (lihat [`Propagator::apply`]).
    pub fn propagator(self, t: f32) -> Propagator {
        if !t.is_finite() || t <= 0.0 {
            return Propagator::IDENTITY;
        }
        let (w, z) = (self.omega, self.zeta);
        if (z - 1.0).abs() <= CRITICAL_BAND {
            // Critically damped: x(t) = e^{-ωt}(x₀ + (v₀ + ω x₀) t).
            let e = (-w * t).exp();
            Propagator {
                xx: e * (1.0 + w * t),
                xv: e * t,
                vx: -e * w * w * t,
                vv: e * (1.0 - w * t),
            }
        } else if z < 1.0 {
            // Underdamped: amplop e^{-ζωt} mengalikan osilasi ω_d.
            let wd = w * (1.0 - z * z).sqrt();
            let e = (-z * w * t).exp();
            let (sin, cos) = (wd * t).sin_cos();
            Propagator {
                xx: e * (cos + (z * w / wd) * sin),
                xv: e * (sin / wd),
                vx: -e * (w * w / wd) * sin,
                vv: e * (cos - (z * w / wd) * sin),
            }
        } else {
            // Overdamped: dua eksponensial murni, akar r₁ (lambat) dan r₂.
            let root = w * (z * z - 1.0).sqrt();
            let r1 = -z * w + root;
            let r2 = -z * w - root;
            let d = r1 - r2;
            let e1 = (r1 * t).exp();
            let e2 = (r2 * t).exp();
            Propagator {
                xx: (-r2 * e1 + r1 * e2) / d,
                xv: (e1 - e2) / d,
                vx: (w * w) * (e2 - e1) / d,
                vv: (r1 * e1 - r2 * e2) / d,
            }
        }
    }

    /// Selesaikan keadaan skalar setelah `t` detik.
    ///
    /// `x0` adalah **simpangan terhadap target**, bukan posisi absolut.
    /// Mengembalikan `(simpangan, kecepatan)` baru.
    pub fn solve(self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        self.propagator(t).apply(x0, v0)
    }

    /// **Batas atas** waktu (detik) sampai simpangan `x0` dengan kecepatan
    /// `v0` masuk ke dalam `tolerance`.
    ///
    /// Dipakai untuk uji dan diagnostik; mesin animasi sendiri tidak
    /// membutuhkannya — ia berhenti karena keadaannya sudah cukup dekat, bukan
    /// karena jam habis.
    ///
    /// Kenapa batas atas dan bukan angka persis: syarat berhenti
    /// ("cukup dekat **dan** cukup pelan") tidak monoton terhadap waktu pada
    /// spring yang memantul — kecepatan menyentuh nol di setiap puncak
    /// pantulan, jadi ada pulau-pulau waktu yang lolos lebih awal. Yang
    /// **monoton** adalah energi `ω²x² + v²`: turunannya `−4ζω v² ≤ 0` di
    /// ketiga rezim. Taksiran di sini memakai energi itu, sehingga jawabannya
    /// tidak pernah lebih kecil dari waktu berhenti yang sebenarnya.
    pub fn settling_time(self, x0: f32, v0: f32, tolerance: Tolerance) -> f32 {
        let w = self.omega;
        // Ambang energi (dalam satuan kecepatan) yang menjamin kedua syarat
        // toleransi sekaligus terpenuhi.
        let batas = (w * tolerance.distance).min(tolerance.velocity);
        let energi = |x: f32, v: f32| ((w * x) * (w * x) + v * v).sqrt();
        if energi(x0, v0) <= batas {
            return 0.0;
        }
        let mut hi = (1.0 / (self.zeta * w)).max(MIN_DURATION);
        for _ in 0..64 {
            let (x, v) = self.solve(x0, v0, hi);
            if energi(x, v) <= batas {
                break;
            }
            hi *= 2.0;
        }
        let mut lo = 0.0;
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            let (x, v) = self.solve(x0, v0, mid);
            if energi(x, v) <= batas {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }
}

impl Default for Spring {
    fn default() -> Self {
        Self::smooth()
    }
}

/// Matriks 2×2 yang memindahkan `(simpangan, kecepatan)` maju `t` detik.
///
/// Dihasilkan [`Spring::propagator`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Propagator {
    /// Kontribusi simpangan awal ke simpangan baru.
    pub xx: f32,
    /// Kontribusi kecepatan awal ke simpangan baru.
    pub xv: f32,
    /// Kontribusi simpangan awal ke kecepatan baru.
    pub vx: f32,
    /// Kontribusi kecepatan awal ke kecepatan baru.
    pub vv: f32,
}

impl Propagator {
    /// Perambatan nol detik: keadaan tidak berubah.
    pub const IDENTITY: Self = Self {
        xx: 1.0,
        xv: 0.0,
        vx: 0.0,
        vv: 1.0,
    };

    /// Terapkan ke sepasang nilai apa pun yang bisa dianimasikan.
    ///
    /// Skalar, [`silka_paint::Point`], [`silka_paint::Size`], dan
    /// [`silka_paint::Color`] memakai jalur yang sama persis.
    pub fn apply<T: Animatable>(self, x0: T, v0: T) -> (T, T) {
        (
            x0.scale(self.xx).add(v0.scale(self.xv)),
            x0.scale(self.vx).add(v0.scale(self.vv)),
        )
    }

    /// Benar bila semua koefisien berhingga.
    pub fn is_finite(self) -> bool {
        self.xx.is_finite() && self.xv.is_finite() && self.vx.is_finite() && self.vv.is_finite()
    }
}
