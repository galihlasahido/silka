//! **Sistem animasi spring** — jantung "rasa Apple" (REKOMENDASI §3.5).
//!
//! Empat keputusan yang mengikat, langsung dari WWDC23 "Animate with springs":
//!
//! 1. **Spring adalah kurva default**, bukan ease-in-out. Parameternya
//!    *perceptual*: durasi + bounce ([`Spring`]), dengan preset
//!    [`Spring::smooth`] / [`Spring::snappy`] / [`Spring::bouncy`].
//! 2. **Nilai menyimpan `(posisi, velocity)`** ([`SpringValue`]) — bukan
//!    "berapa persen kurva sudah diputar". Itulah yang membuat setiap animasi
//!    **interruptible**.
//! 3. **Retarget kapan pun sambil membawa velocity**
//!    ([`SpringValue::set_target`]). Ini bukan fitur tambahan melainkan
//!    konsekuensi langsung dari solusi closed-form: setiap frame diselesaikan
//!    dari keadaan *sekarang*, jadi tidak ada animasi lama yang perlu
//!    dibatalkan. Handoff gesture (fling → spring) tinggal menyuntikkan
//!    velocity lewat [`SpringValue::set_velocity`] — atau, untuk gerakan 2D,
//!    menyerahkan [`Velocity`](crate::input::Velocity) dari velocity tracker
//!    apa adanya lewat [`SpringValue::hand_off`].
//! 4. **Reduced-motion dihormati** ([`Motion`]) — bukan dipoles belakangan.
//!
//! Solusinya **closed-form damped harmonic oscillator**, bukan integrasi
//! numerik per frame. Konsekuensi praktisnya besar: hasilnya tidak bergantung
//! pada ukuran langkah, frame yang di-drop tidak menggeser animasi, `dt` besar
//! tidak bisa membuat integrator meledak, dan satu matriks 2×2
//! ([`Propagator`]) melayani skalar maupun vektor sekaligus.
//!
//! ## Sambungan ke scheduler
//!
//! Animasi **tidak** memakai timer yang berdetak. [`AnimationDriver`] membagikan
//! sebuah [`Tick`] selama frame; nilai yang masih bergerak menandai dirinya di
//! situ, dan hanya kalau ada tanda itulah [`AnimationDriver::end_frame`]
//! mengembalikan [`Dirty::ANIMATION`](crate::scheduler::Dirty::ANIMATION)
//! untuk diteruskan ke
//! [`FrameScheduler::request`](crate::scheduler::FrameScheduler::request).
//! Begitu semua spring settle, scheduler kembali idle dan GPU benar-benar tidur
//! (§3.5 "render hanya saat dirty").
//!
//! ```
//! use std::time::{Duration, Instant};
//! use rustui_core::animation::{AnimationDriver, Spring, SpringValue};
//! use rustui_core::scheduler::Dirty;
//!
//! let mut driver = AnimationDriver::new();
//! let mut offset = SpringValue::new(0.0).with_spring(Spring::snappy());
//!
//! // Interaksi mengarahkan nilai ke tujuan baru.
//! offset.set_target(64.0);
//!
//! let mut now = Instant::now();
//! let mut dirty = Dirty::ANIMATION;
//! while dirty.contains(Dirty::ANIMATION) {
//!     let tick = driver.begin_frame(now);
//!     let _y = tick.advance(&mut offset);
//!     dirty = driver.end_frame(tick);
//!     now += Duration::from_micros(8_333); // 120 Hz dari display link
//! }
//! assert_eq!(offset.position(), 64.0);
//! ```

mod driver;
mod motion;
mod spring;
#[cfg(test)]
mod tests;
mod value;

pub use driver::{AnimationDriver, Tick};
pub use motion::{Motion, MotionRole};
pub use spring::{Propagator, Spring, MAX_BOUNCE, MIN_DURATION};
pub use value::{Animatable, SpringValue, Tolerance};
