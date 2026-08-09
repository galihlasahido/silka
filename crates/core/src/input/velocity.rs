//! Velocity tracker — **prasyarat gesture handoff** (REKOMENDASI §3.5).
//!
//! Janji §3.5: "semua animasi harus interruptible; nilai animasi menyimpan
//! `(posisi, velocity)` dan bisa di-retarget kapan saja sambil membawa
//! velocity — gesture handoff (fling → spring) butuh velocity tracker di input
//! layer". Modul ini adalah bagian "di input layer"-nya: ia mengubah rentetan
//! sampel posisi menjadi satu angka kecepatan yang bisa diserahkan ke spring
//! saat jari diangkat.
//!
//! Cara menaksirnya mengikuti Flutter (`VelocityTracker`): **regresi kuadrat
//! terkecil berderajat dua** atas sampel dalam jendela waktu pendek. Kenapa
//! bukan sekadar `(p₁ − p₀) / Δt`: dua sampel terakhir sangat berisik, dan
//! justru di akhir gesture-lah jari biasanya melambat — turunan dari fit
//! kuadratik menangkap perlambatan itu, sedangkan beda hingga menangkap
//! kebisingan.
//!
//! ```
//! use std::time::Duration;
//! use silka_core::input::VelocityTracker;
//! use silka_paint::Point;
//!
//! let mut t = VelocityTracker::new();
//! // Bergerak 600 poin/detik ke bawah selama 50 ms.
//! for i in 0..6 {
//!     let ms = i * 10;
//!     t.add(Duration::from_millis(ms), Point::new(0.0, 0.6 * ms as f32));
//! }
//! let v = t.velocity();
//! assert!((v.y - 600.0).abs() < 1.0, "v = {v:?}");
//! ```

use std::collections::VecDeque;
use std::time::Duration;

use silka_paint::Point;

/// Jendela waktu sampel yang ikut diperhitungkan.
///
/// Lebih panjang = lebih halus tapi lamban bereaksi pada perubahan arah;
/// 100 ms adalah angka yang dipakai Flutter dan Android.
pub const HORIZON: Duration = Duration::from_millis(100);

/// Jumlah sampel maksimum yang disimpan.
pub const MAX_SAMPLES: usize = 20;

/// Kecepatan dalam **poin logis per detik**.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocity {
    /// Komponen horizontal.
    pub x: f32,
    /// Komponen vertikal (positif = ke bawah).
    pub y: f32,
}

impl Velocity {
    /// Diam.
    pub const ZERO: Velocity = Velocity { x: 0.0, y: 0.0 };

    /// Kecepatan baru.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Besaran (panjang vektor).
    pub fn magnitude(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Versi yang besarannya dibatasi `max`, arah dipertahankan.
    ///
    /// Wajib dipakai sebelum menyerahkan ke spring: satu sampel gila dari
    /// driver trackpad tidak boleh melempar konten ribuan poin.
    pub fn clamp_magnitude(self, max: f32) -> Self {
        let m = self.magnitude();
        if m <= max || m == 0.0 {
            return self;
        }
        let k = max / m;
        Velocity::new(self.x * k, self.y * k)
    }

    /// Benar bila cukup cepat untuk dianggap lemparan, bukan sekadar lepas.
    pub fn is_fling(self, min_speed: f32) -> bool {
        self.magnitude() >= min_speed
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    time: Duration,
    position: Point,
}

/// Pelacak kecepatan satu penunjuk.
///
/// Satu instance per [`crate::input::PointerId`]; router membuat dan
/// membuangnya mengikuti hidup penunjuk.
#[derive(Debug, Clone, Default)]
pub struct VelocityTracker {
    samples: VecDeque<Sample>,
}

impl VelocityTracker {
    /// Pelacak kosong.
    pub fn new() -> Self {
        Self::default()
    }

    /// Kosongkan riwayat — dipanggil di awal setiap gesture baru.
    ///
    /// Tanpa ini, jari yang menyentuh lagi setelah jeda panjang akan mewarisi
    /// kecepatan gesture sebelumnya.
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// Jumlah sampel tersimpan.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Benar bila belum ada sampel.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Catat satu posisi.
    ///
    /// Sampel yang mundur ke belakang (jam melompat, event terlambat) memulai
    /// riwayat baru alih-alih menghasilkan kecepatan negatif palsu.
    pub fn add(&mut self, time: Duration, position: Point) {
        if let Some(terakhir) = self.samples.back() {
            if time < terakhir.time {
                self.samples.clear();
            }
        }
        self.samples.push_back(Sample { time, position });
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
        let batas = time.saturating_sub(HORIZON);
        while self.samples.len() > 2 && self.samples.front().is_some_and(|s| s.time < batas) {
            self.samples.pop_front();
        }
    }

    /// Taksiran kecepatan pada sampel terakhir.
    ///
    /// [`Velocity::ZERO`] bila sampelnya belum cukup — bukan tebakan.
    pub fn velocity(&self) -> Velocity {
        let sampel: Vec<&Sample> = self.samples.iter().collect();
        if sampel.len() < 2 {
            return Velocity::ZERO;
        }
        let akhir = sampel[sampel.len() - 1];
        // Waktu relatif terhadap sampel terakhir (≤ 0), dalam detik. Dengan
        // begitu kecepatan yang dicari adalah koefisien linier di t = 0.
        let t: Vec<f32> = sampel
            .iter()
            .map(|s| -(akhir.time.saturating_sub(s.time).as_secs_f32()))
            .collect();
        let x: Vec<f32> = sampel.iter().map(|s| s.position.x).collect();
        let y: Vec<f32> = sampel.iter().map(|s| s.position.y).collect();
        Velocity::new(turunan_di_nol(&t, &x), turunan_di_nol(&t, &y))
    }

    /// Kecepatan yang sudah dibatasi besarannya.
    pub fn velocity_clamped(&self, max: f32) -> Velocity {
        self.velocity().clamp_magnitude(max)
    }
}

/// Koefisien linier `c₁` dari fit `p(t) = c₀ + c₁t + c₂t²`, yaitu kecepatan di
/// `t = 0` (sampel terakhir).
///
/// Turun sendiri ke fit linier saat sampelnya kurang dari tiga atau saat
/// sistem persamaannya singular (semua sampel pada waktu yang sama).
fn turunan_di_nol(t: &[f32], p: &[f32]) -> f32 {
    debug_assert_eq!(t.len(), p.len());
    if t.len() >= 3 {
        if let Some(c) = kuadrat_terkecil::<3>(t, p) {
            return c[1];
        }
    }
    match kuadrat_terkecil::<2>(t, p) {
        Some(c) => c[1],
        None => 0.0,
    }
}

/// Kuadrat terkecil untuk polinom berderajat `N-1` lewat persamaan normal.
///
/// `N` kecil (2 atau 3), jadi eliminasi Gauss dengan pivot parsial di matriks
/// `N×N` sudah lebih dari cukup — dan bebas alokasi.
fn kuadrat_terkecil<const N: usize>(t: &[f32], p: &[f32]) -> Option<[f32; N]> {
    if t.len() < N {
        return None;
    }
    // Persamaan normal: (AᵀA)c = Aᵀp dengan A_ij = t_i^j.
    let mut a = [[0.0f64; N]; N];
    let mut b = [0.0f64; N];
    for (ti, pi) in t.iter().zip(p.iter()) {
        let ti = *ti as f64;
        let mut pangkat = [0.0f64; N];
        let mut v = 1.0f64;
        for slot in pangkat.iter_mut() {
            *slot = v;
            v *= ti;
        }
        for j in 0..N {
            for k in 0..N {
                a[j][k] += pangkat[j] * pangkat[k];
            }
            b[j] += pangkat[j] * *pi as f64;
        }
    }

    // Eliminasi Gauss dengan pivot parsial.
    for i in 0..N {
        let mut pivot = i;
        for r in (i + 1)..N {
            if a[r][i].abs() > a[pivot][i].abs() {
                pivot = r;
            }
        }
        if a[pivot][i].abs() < 1e-12 {
            return None;
        }
        a.swap(i, pivot);
        b.swap(i, pivot);
        for r in (i + 1)..N {
            let f = a[r][i] / a[i][i];
            if f == 0.0 {
                continue;
            }
            let baris = a[i];
            for (c, nilai) in a[r].iter_mut().enumerate().skip(i) {
                *nilai -= f * baris[c];
            }
            b[r] -= f * b[i];
        }
    }

    let mut x = [0.0f64; N];
    for i in (0..N).rev() {
        let mut s = b[i];
        for c in (i + 1)..N {
            s -= a[i][c] * x[c];
        }
        x[i] = s / a[i][i];
    }

    let mut out = [0.0f32; N];
    for i in 0..N {
        if !x[i].is_finite() {
            return None;
        }
        out[i] = x[i] as f32;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn tanpa_sampel_kecepatannya_nol() {
        let t = VelocityTracker::new();
        assert_eq!(t.velocity(), Velocity::ZERO);
        assert!(t.is_empty());
    }

    #[test]
    fn satu_sampel_belum_cukup() {
        let mut t = VelocityTracker::new();
        t.add(ms(0), Point::new(0.0, 0.0));
        assert_eq!(t.velocity(), Velocity::ZERO);
    }

    #[test]
    fn dua_sampel_memberi_beda_hingga() {
        let mut t = VelocityTracker::new();
        t.add(ms(0), Point::new(0.0, 0.0));
        t.add(ms(10), Point::new(5.0, -2.0));
        let v = t.velocity();
        assert!((v.x - 500.0).abs() < 0.5, "{v:?}");
        assert!((v.y + 200.0).abs() < 0.5, "{v:?}");
    }

    #[test]
    fn gerak_lurus_beraturan_terbaca_persis() {
        let mut t = VelocityTracker::new();
        for i in 0..8 {
            let waktu = i * 8;
            t.add(
                ms(waktu),
                Point::new(-0.25 * waktu as f32, 1.2 * waktu as f32),
            );
        }
        let v = t.velocity();
        assert!((v.x + 250.0).abs() < 1.0, "{v:?}");
        assert!((v.y - 1200.0).abs() < 1.0, "{v:?}");
    }

    #[test]
    fn perlambatan_terbaca_bukan_rata_rata() {
        // p(t) = v₀t + ½at² dengan v₀ = 1000, a = −4000 → di t = 60 ms
        // kecepatan sesungguhnya 760, sedangkan rata-rata sepanjang gerak 880.
        let mut t = VelocityTracker::new();
        for i in 0..=6 {
            let detik = i as f32 * 0.01;
            let p = 1000.0 * detik - 2000.0 * detik * detik;
            t.add(ms(i * 10), Point::new(0.0, p));
        }
        let v = t.velocity();
        assert!(
            (v.y - 760.0).abs() < 5.0,
            "fit kuadratik harus menangkap perlambatan, dapat {v:?}"
        );
    }

    #[test]
    fn sampel_di_luar_horizon_dibuang() {
        let mut t = VelocityTracker::new();
        // Gerakan lama yang cepat…
        for i in 0..5 {
            t.add(ms(i * 5), Point::new(0.0, 10.0 * i as f32));
        }
        // …lalu jeda panjang dan gerakan lambat.
        for i in 0..5 {
            let waktu = 500 + i * 10;
            t.add(ms(waktu), Point::new(0.0, 100.0 + i as f32));
        }
        assert!(t.len() <= 5, "sampel lama harus terbuang: {}", t.len());
        let v = t.velocity();
        assert!(v.y.abs() < 200.0, "kecepatan lama tidak boleh bocor: {v:?}");
    }

    #[test]
    fn diam_di_tempat_berarti_nol() {
        let mut t = VelocityTracker::new();
        for i in 0..6 {
            t.add(ms(i * 10), Point::new(40.0, 12.0));
        }
        assert!(t.velocity().magnitude() < 1e-3);
    }

    #[test]
    fn waktu_mundur_memulai_riwayat_baru() {
        let mut t = VelocityTracker::new();
        for i in 0..5 {
            t.add(ms(100 + i * 10), Point::new(0.0, 10.0 * i as f32));
        }
        t.add(ms(0), Point::new(0.0, 0.0));
        assert_eq!(t.len(), 1);
        assert_eq!(t.velocity(), Velocity::ZERO);
    }

    #[test]
    fn reset_membuang_gesture_sebelumnya() {
        let mut t = VelocityTracker::new();
        for i in 0..5 {
            t.add(ms(i * 10), Point::new(0.0, 20.0 * i as f32));
        }
        assert!(t.velocity().magnitude() > 100.0);
        t.reset();
        assert_eq!(t.velocity(), Velocity::ZERO);
    }

    #[test]
    fn jumlah_sampel_dibatasi() {
        let mut t = VelocityTracker::new();
        for i in 0..(MAX_SAMPLES as u64 * 3) {
            // Waktu rapat supaya bukan horizon yang membuangnya.
            t.add(Duration::from_micros(i * 200), Point::new(i as f32, 0.0));
        }
        assert!(t.len() <= MAX_SAMPLES);
    }

    #[test]
    fn clamp_menjaga_arah() {
        let v = Velocity::new(300.0, 400.0); // besaran 500
        let c = v.clamp_magnitude(100.0);
        assert!((c.magnitude() - 100.0).abs() < 1e-3);
        assert!((c.x / c.y - 0.75).abs() < 1e-4);
        // Yang sudah di bawah batas tidak disentuh.
        assert_eq!(
            Velocity::new(3.0, 4.0).clamp_magnitude(100.0),
            Velocity::new(3.0, 4.0)
        );
        assert_eq!(Velocity::ZERO.clamp_magnitude(10.0), Velocity::ZERO);
    }

    #[test]
    fn ambang_fling() {
        assert!(Velocity::new(0.0, 900.0).is_fling(300.0));
        assert!(!Velocity::new(0.0, 100.0).is_fling(300.0));
    }
}
