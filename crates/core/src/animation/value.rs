//! Nilai teranimasi: `(posisi, velocity)` yang bisa di-retarget kapan saja.

use std::time::Duration;

use rustui_paint::{Color, Insets, Point, Rect, Size};

use super::motion::{Motion, MotionRole};
use super::spring::Spring;

// ---------------------------------------------------------------------------
// Tolerance
// ---------------------------------------------------------------------------

/// Seberapa dekat ke target sudah boleh disebut "selesai".
///
/// Secara matematis spring tidak pernah benar-benar sampai — ia mendekat
/// selamanya. Yang menentukan kapan renderer boleh kembali tidur adalah
/// toleransi ini, dan karena itu ia bagian dari kontrak, bukan konstanta
/// tersembunyi: satuan posisi berbeda antara poin logis dan kanal warna.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Jarak maksimum ke target yang masih dianggap sampai.
    pub distance: f32,
    /// Laju maksimum yang masih dianggap diam (satuan posisi per detik).
    pub velocity: f32,
}

impl Tolerance {
    /// Toleransi untuk besaran dalam **poin logis** (posisi, ukuran, radius).
    ///
    /// 1/512 poin jauh di bawah satu piksel fisik bahkan di layar 3×, jadi
    /// berhenti di sini tidak pernah terlihat.
    pub const POINTS: Self = Self {
        distance: 1.0 / 512.0,
        velocity: 1.0 / 512.0,
    };

    /// Toleransi untuk kanal warna 0..1 — di bawah satu langkah 8-bit
    /// (1/255 ≈ 0,0039).
    pub const COLOR: Self = Self {
        distance: 1.0 / 2048.0,
        velocity: 1.0 / 2048.0,
    };

    /// Toleransi kustom.
    pub fn new(distance: f32, velocity: f32) -> Self {
        Self {
            distance: distance.abs(),
            velocity: velocity.abs(),
        }
    }

    /// Benar bila simpangan **dan** laju sudah cukup kecil.
    ///
    /// Keduanya wajib: nilai yang kebetulan melintasi target dengan kecepatan
    /// penuh belum selesai, dan nilai yang berhenti jauh dari target juga
    /// belum.
    pub fn settled(self, distance: f32, speed: f32) -> bool {
        distance <= self.distance && speed <= self.velocity
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Self::POINTS
    }
}

// ---------------------------------------------------------------------------
// Animatable
// ---------------------------------------------------------------------------

/// Nilai yang bisa dijalankan oleh spring.
///
/// Cukup ruang vektor: penjumlahan, pengurangan, perkalian skalar, dan sebuah
/// besaran untuk menguji konvergensi. Karena solusinya linear
/// ([`super::Propagator`]), semua komponen memakai koefisien yang sama — tidak
/// ada spring terpisah per sumbu yang bisa keluar dari sinkron.
pub trait Animatable: Copy + std::fmt::Debug {
    /// Toleransi yang masuk akal untuk satuan tipe ini.
    const TOLERANCE: Tolerance;

    /// Elemen nol (dipakai sebagai kecepatan diam).
    fn zero() -> Self;

    /// Penjumlahan komponen demi komponen.
    fn add(self, other: Self) -> Self;

    /// Pengurangan komponen demi komponen.
    fn sub(self, other: Self) -> Self;

    /// Perkalian dengan skalar.
    fn scale(self, k: f32) -> Self;

    /// Besaran (norm Euclid) — dipakai untuk menguji `settled`.
    fn magnitude(self) -> f32;

    /// Benar bila seluruh komponen berhingga.
    fn is_finite(self) -> bool;
}

impl Animatable for f32 {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        0.0
    }
    fn add(self, other: Self) -> Self {
        self + other
    }
    fn sub(self, other: Self) -> Self {
        self - other
    }
    fn scale(self, k: f32) -> Self {
        self * k
    }
    fn magnitude(self) -> f32 {
        self.abs()
    }
    fn is_finite(self) -> bool {
        f32::is_finite(self)
    }
}

impl Animatable for Point {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Point::ZERO
    }
    fn add(self, other: Self) -> Self {
        Point::new(self.x + other.x, self.y + other.y)
    }
    fn sub(self, other: Self) -> Self {
        Point::new(self.x - other.x, self.y - other.y)
    }
    fn scale(self, k: f32) -> Self {
        Point::new(self.x * k, self.y * k)
    }
    fn magnitude(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Animatable for Size {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Size::ZERO
    }
    fn add(self, other: Self) -> Self {
        Size::new(self.width + other.width, self.height + other.height)
    }
    fn sub(self, other: Self) -> Self {
        Size::new(self.width - other.width, self.height - other.height)
    }
    fn scale(self, k: f32) -> Self {
        Size::new(self.width * k, self.height * k)
    }
    fn magnitude(self) -> f32 {
        (self.width * self.width + self.height * self.height).sqrt()
    }
    fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }
}

impl Animatable for Insets {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Insets::ZERO
    }
    fn add(self, other: Self) -> Self {
        Insets {
            top: self.top + other.top,
            right: self.right + other.right,
            bottom: self.bottom + other.bottom,
            left: self.left + other.left,
        }
    }
    fn sub(self, other: Self) -> Self {
        Insets {
            top: self.top - other.top,
            right: self.right - other.right,
            bottom: self.bottom - other.bottom,
            left: self.left - other.left,
        }
    }
    fn scale(self, k: f32) -> Self {
        Insets {
            top: self.top * k,
            right: self.right * k,
            bottom: self.bottom * k,
            left: self.left * k,
        }
    }
    fn magnitude(self) -> f32 {
        (self.top * self.top
            + self.right * self.right
            + self.bottom * self.bottom
            + self.left * self.left)
            .sqrt()
    }
    fn is_finite(self) -> bool {
        self.top.is_finite()
            && self.right.is_finite()
            && self.bottom.is_finite()
            && self.left.is_finite()
    }
}

impl Animatable for Rect {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Rect::new(0.0, 0.0, 0.0, 0.0)
    }
    fn add(self, other: Self) -> Self {
        Rect::from_origin_size(self.origin.add(other.origin), self.size.add(other.size))
    }
    fn sub(self, other: Self) -> Self {
        Rect::from_origin_size(self.origin.sub(other.origin), self.size.sub(other.size))
    }
    fn scale(self, k: f32) -> Self {
        Rect::from_origin_size(self.origin.scale(k), self.size.scale(k))
    }
    fn magnitude(self) -> f32 {
        (self.origin.magnitude().powi(2) + self.size.magnitude().powi(2)).sqrt()
    }
    fn is_finite(self) -> bool {
        Animatable::is_finite(self.origin) && Animatable::is_finite(self.size)
    }
}

impl Animatable for Color {
    const TOLERANCE: Tolerance = Tolerance::COLOR;

    fn zero() -> Self {
        Color::srgba(0.0, 0.0, 0.0, 0.0)
    }
    fn add(self, other: Self) -> Self {
        Color::srgba(
            self.r + other.r,
            self.g + other.g,
            self.b + other.b,
            self.a + other.a,
        )
    }
    fn sub(self, other: Self) -> Self {
        Color::srgba(
            self.r - other.r,
            self.g - other.g,
            self.b - other.b,
            self.a - other.a,
        )
    }
    fn scale(self, k: f32) -> Self {
        Color::srgba(self.r * k, self.g * k, self.b * k, self.a * k)
    }
    fn magnitude(self) -> f32 {
        (self.r * self.r + self.g * self.g + self.b * self.b + self.a * self.a).sqrt()
    }
    fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }
}

// ---------------------------------------------------------------------------
// SpringValue
// ---------------------------------------------------------------------------

/// Nilai yang dijalankan spring, menyimpan **posisi dan velocity**.
///
/// Inilah unit animasi framework (REKOMENDASI §3.5). Dua sifat yang mengikat:
///
/// - **Selalu interruptible.** Tidak ada "durasi tersisa" atau kurva yang harus
///   diputar sampai habis; yang tersimpan hanya keadaan sekarang. Karena itu
///   [`SpringValue::set_target`] boleh dipanggil di tengah gerakan berapa kali
///   pun — velocity ikut terbawa dan tidak ada patahan yang terlihat (WWDC23).
/// - **Berhenti benar-benar berhenti.** Begitu keadaan masuk toleransi, nilai
///   dikunci ke target dan [`SpringValue::is_animating`] menjadi `false`,
///   sehingga scheduler bisa kembali tidur (§3.5 "render hanya saat dirty").
///
/// ```
/// use std::time::Duration;
/// use rustui_core::animation::{Motion, Spring, SpringValue};
///
/// let mut x = SpringValue::new(0.0).with_spring(Spring::smooth());
/// x.set_target(100.0);
///
/// let dt = Duration::from_micros(8_333); // 120 Hz — datang dari display link
/// while x.is_animating() {
///     x.advance(dt, Motion::Full);
/// }
/// assert_eq!(x.position(), 100.0);
/// assert_eq!(x.velocity(), 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringValue<T: Animatable = f32> {
    spring: Spring,
    role: MotionRole,
    tolerance: Tolerance,
    position: T,
    velocity: T,
    target: T,
    animating: bool,
}

impl<T: Animatable> SpringValue<T> {
    /// Nilai yang diam di `value`, memakai [`Spring::smooth`].
    pub fn new(value: T) -> Self {
        Self {
            spring: Spring::smooth(),
            role: MotionRole::Essential,
            tolerance: T::TOLERANCE,
            position: value,
            velocity: T::zero(),
            target: value,
            animating: false,
        }
    }

    /// Pilih spring (biasanya salah satu preset).
    pub fn with_spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Tandai gerakan ini **dekoratif**: reduced-motion akan mematikannya
    /// sepenuhnya, bukan sekadar membuang pantulannya.
    ///
    /// Pakai untuk gerakan yang tidak membawa informasi (parallax, bounce
    /// hiasan). Gerakan yang *menjelaskan* — sheet naik, disclosure membuka —
    /// biarkan [`MotionRole::Essential`] agar tetap terbaca.
    pub fn decorative(mut self) -> Self {
        self.role = MotionRole::Decorative;
        self
    }

    /// Toleransi berhenti kustom.
    pub fn with_tolerance(mut self, tolerance: Tolerance) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Spring yang sedang dipakai.
    pub fn spring(&self) -> Spring {
        self.spring
    }

    /// Ganti spring **tanpa** mengganggu keadaan.
    ///
    /// Posisi dan velocity dibawa apa adanya, jadi mengganti preset di tengah
    /// gerakan pun mulus.
    pub fn set_spring(&mut self, spring: Spring) {
        self.spring = spring;
    }

    /// Peran gerakan terhadap reduced-motion.
    pub fn role(&self) -> MotionRole {
        self.role
    }

    /// Ganti peran gerakan **tanpa** mengganggu keadaan.
    ///
    /// Pasangan `&mut` dari [`Self::decorative`], sama seperti [`Self::set_spring`]
    /// terhadap [`Self::with_spring`]: dibutuhkan di jalur `update` sebuah view,
    /// tempat node sudah terlanjur ada dan tidak bisa dibangun ulang. Posisi dan
    /// velocity dibawa apa adanya, jadi peran boleh berubah di tengah gerakan.
    pub fn set_role(&mut self, role: MotionRole) {
        self.role = role;
    }

    /// Toleransi berhenti.
    pub fn tolerance(&self) -> Tolerance {
        self.tolerance
    }

    /// Nilai sekarang — inilah yang dipakai layout/paint frame ini.
    pub fn position(&self) -> T {
        self.position
    }

    /// Kecepatan sekarang (satuan posisi per detik).
    pub fn velocity(&self) -> T {
        self.velocity
    }

    /// Target yang sedang dituju.
    pub fn target(&self) -> T {
        self.target
    }

    /// Benar bila masih bergerak dan frame berikutnya masih dibutuhkan.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// **Retarget**: arahkan ke tujuan baru sambil membawa velocity.
    ///
    /// Boleh dipanggil kapan saja — saat diam, saat sedang bergerak, bahkan
    /// setiap frame mengikuti jari yang menyeret. Tidak ada animasi yang
    /// "dibatalkan": yang berubah hanya ke mana keadaan sekarang menuju.
    pub fn set_target(&mut self, target: T) {
        if !target.is_finite() {
            return;
        }
        self.target = target;
        let jarak = self.position.sub(target).magnitude();
        let laju = self.velocity.magnitude();
        if self.tolerance.settled(jarak, laju) {
            self.settle();
        } else {
            self.animating = true;
        }
    }

    /// Lompat ke `value` seketika: posisi, target, dan velocity di-reset.
    ///
    /// Untuk perubahan yang bukan animasi — memuat state baru, berpindah
    /// halaman, membangun ulang widget dari nol.
    pub fn jump_to(&mut self, value: T) {
        self.position = value;
        self.target = value;
        self.velocity = T::zero();
        self.animating = false;
    }

    /// Setel velocity langsung.
    ///
    /// Jalur **handoff gesture** (§3.5): velocity tracker di lapisan input
    /// menyerahkan kecepatan jari saat dilepas, lalu spring meneruskannya —
    /// fling berubah jadi spring tanpa patahan.
    pub fn set_velocity(&mut self, velocity: T) {
        if !velocity.is_finite() {
            return;
        }
        self.velocity = velocity;
        let jarak = self.position.sub(self.target).magnitude();
        if !self.tolerance.settled(jarak, velocity.magnitude()) {
            self.animating = true;
        }
    }

    /// Tambahkan velocity ke yang sudah ada (dorongan berturut-turut).
    pub fn add_velocity(&mut self, delta: T) {
        self.set_velocity(self.velocity.add(delta));
    }

    /// Majukan `dt`; mengembalikan `true` bila masih butuh frame berikutnya.
    ///
    /// `motion` datang dari setting aksesibilitas OS. Tidak ada penjepitan
    /// `dt` di sini dan itu disengaja: solusi closed-form tidak bisa meledak
    /// oleh langkah besar seperti integrator numerik — `dt` sepuluh detik
    /// cuma berarti nilainya mendarat di target, yang memang jawaban benarnya.
    pub fn advance(&mut self, dt: Duration, motion: Motion) -> bool {
        if !self.animating {
            return false;
        }
        if motion.suppresses(self.role) {
            self.settle();
            return false;
        }
        let dt = dt.as_secs_f32();
        if dt <= 0.0 {
            // Frame pertama sebuah animasi ber-`dt` nol (lihat
            // `AnimationDriver::begin_frame`): belum bergerak, tapi jelas
            // masih butuh frame berikutnya.
            return true;
        }

        let spring = motion.spring(self.spring);
        let p = spring.propagator(dt);
        if !p.is_finite() {
            // Tidak boleh ada NaN yang menjalar ke layout.
            self.settle();
            return false;
        }
        let (x, v) = p.apply(self.position.sub(self.target), self.velocity);
        if !x.is_finite() || !v.is_finite() || self.tolerance.settled(x.magnitude(), v.magnitude())
        {
            self.settle();
            return false;
        }
        self.position = self.target.add(x);
        self.velocity = v;
        true
    }

    /// Selesaikan seketika: posisi = target, velocity = 0, berhenti animasi.
    pub fn settle(&mut self) {
        self.position = self.target;
        self.velocity = T::zero();
        self.animating = false;
    }

    /// **Batas atas** sisa waktu sampai berhenti, dengan `motion` yang berlaku.
    ///
    /// Untuk diagnostik dan uji — mesin animasi tidak memakainya untuk
    /// memutuskan kapan berhenti. Konservatif dari dua arah: lihat
    /// [`Spring::settling_time`], dan simpangan vektor diproyeksikan ke satu
    /// sumbu lewat besarannya, seolah seluruh kecepatan menjauhi target.
    pub fn settling_duration(&self, motion: Motion) -> Duration {
        if !self.animating {
            return Duration::ZERO;
        }
        if motion.suppresses(self.role) {
            return Duration::ZERO;
        }
        let spring = motion.spring(self.spring);
        let jarak = self.position.sub(self.target).magnitude();
        let laju = self.velocity.magnitude();
        let t = spring.settling_time(jarak, laju, self.tolerance);
        Duration::from_secs_f32(t.max(0.0))
    }
}

impl<T: Animatable + Default> Default for SpringValue<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

// ---------------------------------------------------------------------------
// Handoff gesture -> spring
// ---------------------------------------------------------------------------

/// Kecepatan jari sebagai kecepatan spring.
///
/// [`VelocityTracker`](crate::input::VelocityTracker) melapor dalam poin logis
/// per detik, satuan yang persis sama dengan velocity sebuah
/// `SpringValue<Point>`. Tanpa konversi ini setiap pemanggil menyalin `x`/`y`
/// sendiri — pekerjaan sepele yang justru gampang tertukar sumbunya.
impl From<crate::input::Velocity> for Point {
    fn from(v: crate::input::Velocity) -> Self {
        Point::new(v.x, v.y)
    }
}

impl SpringValue<Point> {
    /// **Handoff fling → spring** (REKOMENDASI §3.5).
    ///
    /// Dipanggil saat jari dilepas: kecepatan dari
    /// [`VelocityTracker::velocity`](crate::input::VelocityTracker::velocity)
    /// diserahkan apa adanya, lalu spring meneruskan gerakannya ke `target`
    /// tanpa patahan. Sama sekali bukan animasi baru — hanya keadaan
    /// `(posisi, velocity)` yang sama dengan velocity yang disuntik.
    ///
    /// Batasi besarannya lebih dulu dengan
    /// [`Velocity::clamp_magnitude`](crate::input::Velocity::clamp_magnitude):
    /// satu sampel gila dari driver trackpad tidak boleh melempar konten
    /// ribuan poin.
    ///
    /// ```
    /// use rustui_core::animation::SpringValue;
    /// use rustui_core::input::Velocity;
    /// use rustui_paint::Point;
    ///
    /// let mut offset = SpringValue::new(Point::new(0.0, 0.0));
    /// offset.set_target(Point::new(0.0, -320.0));
    /// offset.hand_off(Velocity::new(0.0, -1800.0).clamp_magnitude(4000.0));
    /// assert_eq!(offset.velocity(), Point::new(0.0, -1800.0));
    /// assert!(offset.is_animating());
    /// ```
    pub fn hand_off(&mut self, velocity: crate::input::Velocity) {
        self.set_velocity(Point::from(velocity));
    }
}
