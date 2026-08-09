//! Fisika guliran: **rubber band ala macOS**, geometri scrollbar, dan langkah
//! keyboard — semuanya fungsi murni.
//!
//! Dipisah dari node-nya dengan sengaja. Yang ada di sini adalah satu-satunya
//! bagian `scroll_view` yang bisa salah secara *diam-diam*: sebuah pantulan
//! yang terlalu keras, thumb yang meleset setengah piksel dari jarinya, atau
//! guliran yang menyangkut satu poin sebelum ujung. Sebagai fungsi murni ia
//! bisa diuji tanpa pohon, tanpa GPU, dan tanpa jam (REKOMENDASI §9.5).
//!
//! ## Rubber band
//!
//! Rumus Apple yang dipakai UIScrollView:
//!
//! ```text
//! f(x) = L · x / (x + L)        L = koefisien × ukuran viewport
//! ```
//!
//! `x` adalah jarak tarik mentah dan `f(x)` simpangan yang terlihat: mulai
//! 1:1 di bawah jari, lalu makin berat, dan tidak pernah melewati `L`.
//! Turunannya — [`rubber_band_factor`] — adalah bentuk yang benar-benar dipakai
//! saat menggulir, karena event guliran datang sebagai **selisih**, bukan
//! sebagai jarak total dari titik tarik:
//!
//! ```text
//! f'(x) = (1 − y/L)²           y = simpangan yang sedang terlihat
//! ```
//!
//! Keduanya konsisten secara matematis, dan ada uji yang membuktikannya dengan
//! mengintegrasikan faktor itu kembali menjadi `f`.

/// Koefisien rubber band ala macOS/UIKit: simpangan maksimum = 0,55 × viewport.
pub const RUBBER_BAND: f32 = 0.55;

/// Ambang perbandingan panjang dalam poin logis.
///
/// Jauh di bawah satu piksel fisik di layar 3×, jadi "sama" di sini berarti
/// sama bagi mata sekaligus stabil terhadap kesalahan pembulatan f32.
const EPS: f32 = 1.0 / 1024.0;

// ---------------------------------------------------------------------------
// Batas guliran
// ---------------------------------------------------------------------------

/// Guliran maksimum yang masih menyisakan isi di layar.
///
/// Nol berarti isi muat seluruhnya — dan itu bukan sekadar angka: wadah yang
/// `max_scroll`-nya nol **tidak boleh** menelan event guliran, supaya wadah di
/// atasnya yang mengambil alih (scroll chaining).
pub fn max_scroll(viewport: f32, content: f32) -> f32 {
    (content - viewport).max(0.0)
}

/// Jepit posisi guliran ke rentang yang sah.
pub fn clamp_scroll(offset: f32, max: f32) -> f32 {
    offset.clamp(0.0, max.max(0.0))
}

/// Simpangan di luar batas: negatif di atas/kiri, positif di bawah/kanan, nol
/// di dalam.
pub fn overshoot(offset: f32, max: f32) -> f32 {
    if offset < 0.0 {
        offset
    } else if offset > max {
        offset - max
    } else {
        0.0
    }
}

/// Tepi terdekat untuk sebuah posisi — tujuan pantulan kembali.
pub fn nearest_bound(offset: f32, max: f32) -> f32 {
    clamp_scroll(offset, max)
}

// ---------------------------------------------------------------------------
// Rubber band
// ---------------------------------------------------------------------------

/// Simpangan maksimum yang diizinkan di luar tepi (`L` pada rumus di atas).
pub fn overscroll_limit(viewport: f32, coefficient: f32) -> f32 {
    (viewport * coefficient).max(0.0)
}

/// Faktor peredam untuk **selisih** guliran saat isi sudah melewati tepi.
///
/// 1 tepat di tepi (bergerak bebas), 0 di simpangan maksimum (tidak bisa
/// ditarik lagi). Inilah `f'` dari rumus Apple, dinyatakan dalam simpangan yang
/// sedang terlihat sehingga tidak perlu mengingat jarak tarik mentah.
pub fn rubber_band_factor(overshoot: f32, viewport: f32, coefficient: f32) -> f32 {
    let limit = overscroll_limit(viewport, coefficient);
    if limit <= 0.0 {
        return 0.0;
    }
    let y = (overshoot.abs() / limit).clamp(0.0, 1.0);
    (1.0 - y) * (1.0 - y)
}

/// Bentuk tertutup rumus Apple: jarak tarik mentah → simpangan yang terlihat.
///
/// Tandanya ikut tanda `raw`.
pub fn rubber_band_offset(raw: f32, viewport: f32, coefficient: f32) -> f32 {
    let limit = overscroll_limit(viewport, coefficient);
    if limit <= 0.0 {
        return 0.0;
    }
    let x = raw.abs();
    (limit * x / (x + limit)).copysign(raw)
}

/// Kebalikan [`rubber_band_offset`]: simpangan yang terlihat → jarak tarik
/// mentah yang menghasilkannya.
///
/// Inilah yang membuat rubber band **bebas dari ukuran langkah**. Event
/// guliran datang sebagai selisih sebesar puluhan poin, bukan sebagai
/// pergerakan infinitesimal; mengalikan selisih sebesar itu dengan
/// [`rubber_band_factor`] akan menyimpang jauh dari kurva (dan di tepi, di mana
/// faktornya masih 1, tidak meredam sama sekali). Jadi yang dilakukan
/// [`apply_delta`] adalah kembali ke jarak tarik mentah, menambahkan
/// selisihnya di sana, lalu memetakannya kembali — hasilnya sama persis dengan
/// menarik jari sejauh itu, seberapa pun kasar sampelnya.
pub fn rubber_band_raw(offset: f32, viewport: f32, coefficient: f32) -> f32 {
    let limit = overscroll_limit(viewport, coefficient);
    let y = offset.abs();
    if limit <= 0.0 || y >= limit {
        return f32::INFINITY.copysign(offset);
    }
    (limit * y / (limit - y)).copysign(offset)
}

/// Terapkan satu selisih guliran, dengan rubber band di luar tepi.
///
/// Tiga aturan yang harus benar bersama, dan urutannya penting:
///
/// 1. Bagian gerakan yang **kembali** ke dalam batas tidak pernah diredam —
///    isi yang sedang melar harus mengikuti jari 1:1 saat ditarik pulang.
/// 2. Bagian yang berada **di dalam** batas bergerak 1:1.
/// 3. Sisanya, yang keluar batas, diredam [`rubber_band_factor`] dan tidak
///    pernah melewati [`overscroll_limit`].
pub fn apply_delta(current: f32, delta: f32, max: f32, viewport: f32, coefficient: f32) -> f32 {
    if !delta.is_finite() || delta == 0.0 {
        return current;
    }
    let max = max.max(0.0);
    let mut pos = current;
    let mut sisa = delta;

    // 1. Pulang dulu: simpangan yang ada dihabiskan tanpa redaman.
    if pos < 0.0 && sisa > 0.0 {
        let langkah = sisa.min(-pos);
        pos += langkah;
        sisa -= langkah;
    } else if pos > max && sisa < 0.0 {
        let langkah = (-sisa).min(pos - max);
        pos -= langkah;
        sisa += langkah;
    }
    if sisa == 0.0 {
        return pos;
    }

    // 2. Di dalam batas: 1:1.
    if pos >= 0.0 && pos <= max {
        let ruang = if sisa > 0.0 { max - pos } else { pos };
        let langkah = sisa.abs().min(ruang) * sisa.signum();
        pos += langkah;
        sisa -= langkah;
    }
    if sisa == 0.0 {
        return pos;
    }

    // 3. Keluar batas: lewat kurva, bukan lewat perkalian per langkah.
    let (batas, arah) = if sisa > 0.0 {
        (max, 1.0f32)
    } else {
        (0.0, -1.0)
    };
    let simpangan = (pos - batas).abs();
    let mentah = rubber_band_raw(simpangan, viewport, coefficient) + sisa.abs();
    let baru = rubber_band_offset(mentah, viewport, coefficient);
    if baru.is_finite() {
        batas + baru * arah
    } else {
        batas + overscroll_limit(viewport, coefficient) * arah
    }
}

// ---------------------------------------------------------------------------
// Scrollbar
// ---------------------------------------------------------------------------

/// Geometri thumb scrollbar pada sumbu guliran, koordinat lokal wadah.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thumb {
    /// Jarak dari tepi awal wadah ke awal thumb.
    pub offset: f32,
    /// Panjang thumb pada sumbu guliran.
    pub length: f32,
}

impl Thumb {
    /// Ujung thumb.
    pub fn end(self) -> f32 {
        self.offset + self.length
    }

    /// Benar bila `pos` (koordinat sumbu guliran) berada di atas thumb.
    pub fn contains(self, pos: f32) -> bool {
        pos >= self.offset && pos <= self.end()
    }
}

/// Geometri thumb untuk keadaan guliran tertentu.
///
/// `None` berarti tidak ada yang bisa digulir — dan berarti juga tidak ada
/// scrollbar yang boleh digambar sama sekali.
///
/// Dua sifat yang ditiru dari macOS:
///
/// - Panjang thumb sebanding dengan **porsi isi yang terlihat**, tapi tidak
///   pernah lebih pendek dari `min_length` (hit target, HIG).
/// - Saat isi melar melewati tepi, thumb **menyusut** menempel di tepi itu —
///   umpan balik yang membuat rubber band terbaca, bukan terasa seperti bug.
pub fn thumb(viewport: f32, content: f32, offset: f32, min_length: f32) -> Option<Thumb> {
    if viewport <= 0.0 || content <= viewport + EPS {
        return None;
    }
    let max = max_scroll(viewport, content);
    let minimum = min_length.clamp(0.0, viewport);
    let ideal = viewport * (viewport / content);
    let mut length = ideal.clamp(minimum, viewport);

    let simpangan = overshoot(offset, max).abs();
    if simpangan > 0.0 {
        length = (length - simpangan).max(minimum);
    }

    let jalur = (viewport - length).max(0.0);
    let posisi = if offset < 0.0 {
        0.0
    } else if offset > max {
        jalur
    } else if max <= 0.0 {
        0.0
    } else {
        (offset / max) * jalur
    };
    Some(Thumb {
        offset: posisi.clamp(0.0, jalur),
        length,
    })
}

/// Kebalikan [`thumb`]: posisi thumb yang diseret → posisi guliran.
///
/// Dipakai saat pengguna menyeret scrollbar langsung. Selalu di dalam batas —
/// menyeret bar tidak pernah menghasilkan rubber band, persis seperti AppKit.
pub fn scroll_for_thumb(viewport: f32, content: f32, thumb_offset: f32, min_length: f32) -> f32 {
    let Some(t) = thumb(viewport, content, 0.0, min_length) else {
        return 0.0;
    };
    let jalur = (viewport - t.length).max(0.0);
    if jalur <= 0.0 {
        return 0.0;
    }
    let max = max_scroll(viewport, content);
    clamp_scroll(thumb_offset / jalur * max, max)
}

// ---------------------------------------------------------------------------
// Langkah keyboard & scroll-to
// ---------------------------------------------------------------------------

/// Berapa jauh satu Page Up/Down menggulir.
///
/// Satu layar penuh **dikurangi satu baris**: konvensi yang sama di macOS,
/// Windows, dan setiap browser — mata butuh satu baris tumpang tindih untuk
/// menemukan kembali tempatnya.
pub fn page_step(viewport: f32, line: f32) -> f32 {
    (viewport - line.max(0.0)).max(viewport * 0.5).max(0.0)
}

/// Posisi guliran terkecil yang membuat rentang `[start, start + extent]`
/// terlihat penuh, dengan `padding` di tepinya.
///
/// Sudah terlihat = tidak bergerak sama sekali. Inilah aturan yang membuat
/// `scroll_into_view` tidak melompat-lompat saat fokus berpindah di antara
/// baris yang sudah sama-sama terlihat.
pub fn scroll_to_reveal(offset: f32, viewport: f32, start: f32, extent: f32, padding: f32) -> f32 {
    let atas = start - padding;
    let bawah = start + extent.max(0.0) + padding;
    if atas < offset {
        atas
    } else if bawah > offset + viewport {
        // Isi yang lebih tinggi dari viewport diratakan ke tepi awalnya:
        // melihat awal sebuah baris panjang selalu lebih berguna daripada
        // melihat ujungnya.
        (bawah - viewport).min(atas)
    } else {
        offset
    }
}

/// Kecepatan guliran dari sepasang sampel event, poin logis per detik.
///
/// Inersia trackpad datang dari OS sebagai rentetan selisih (INTEGRASI-NATIVE
/// §3), bukan sebagai kecepatan. Ini yang mengubahnya kembali menjadi kecepatan
/// supaya bisa diserahkan ke spring saat isi membentur tepi — handoff yang
/// dijanjikan §3.5, hanya dengan sumber yang berbeda dari jari.
pub fn velocity_from(delta: f32, dt: std::time::Duration) -> f32 {
    let dt = dt.as_secs_f32();
    if dt <= 0.0 || !delta.is_finite() {
        return 0.0;
    }
    delta / dt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const VIEWPORT: f32 = 400.0;

    #[test]
    fn max_scroll_nol_saat_isi_muat() {
        assert_eq!(max_scroll(400.0, 300.0), 0.0);
        assert_eq!(max_scroll(400.0, 400.0), 0.0);
        assert_eq!(max_scroll(400.0, 1000.0), 600.0);
    }

    #[test]
    fn simpangan_hanya_di_luar_batas() {
        assert_eq!(overshoot(0.0, 600.0), 0.0);
        assert_eq!(overshoot(300.0, 600.0), 0.0);
        assert_eq!(overshoot(-20.0, 600.0), -20.0);
        assert_eq!(overshoot(640.0, 600.0), 40.0);
        assert_eq!(nearest_bound(-20.0, 600.0), 0.0);
        assert_eq!(nearest_bound(640.0, 600.0), 600.0);
    }

    #[test]
    fn faktor_rubber_band_turun_dari_satu_ke_nol() {
        let f0 = rubber_band_factor(0.0, VIEWPORT, RUBBER_BAND);
        assert!((f0 - 1.0).abs() < 1e-6, "di tepi harus bebas: {f0}");

        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        assert_eq!(rubber_band_factor(limit, VIEWPORT, RUBBER_BAND), 0.0);
        assert_eq!(rubber_band_factor(limit * 2.0, VIEWPORT, RUBBER_BAND), 0.0);

        // Monoton turun, dan tidak pernah keluar dari 0..1.
        let mut sebelumnya = f0;
        for i in 1..=40 {
            let f = rubber_band_factor(limit * i as f32 / 40.0, VIEWPORT, RUBBER_BAND);
            assert!((0.0..=1.0).contains(&f), "faktor liar: {f}");
            assert!(f <= sebelumnya, "faktor naik di langkah {i}");
            sebelumnya = f;
        }
        // Tanda simpangan tidak berpengaruh: melar ke atas dan ke bawah sama.
        assert_eq!(
            rubber_band_factor(-30.0, VIEWPORT, RUBBER_BAND),
            rubber_band_factor(30.0, VIEWPORT, RUBBER_BAND)
        );
    }

    #[test]
    fn viewport_nol_tidak_pernah_menghasilkan_nan() {
        assert_eq!(rubber_band_factor(10.0, 0.0, RUBBER_BAND), 0.0);
        assert_eq!(rubber_band_offset(10.0, 0.0, RUBBER_BAND), 0.0);
        assert_eq!(apply_delta(0.0, 50.0, 0.0, 0.0, RUBBER_BAND), 0.0);
        assert!(thumb(0.0, 1000.0, 0.0, 44.0).is_none());
    }

    #[test]
    fn simpangan_tidak_pernah_melewati_batasnya() {
        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        assert_eq!(limit, VIEWPORT * RUBBER_BAND);
        // Tarikan sebesar apa pun berhenti di limit.
        assert!(rubber_band_offset(1.0e6, VIEWPORT, RUBBER_BAND) < limit);
        assert!(rubber_band_offset(1.0e6, VIEWPORT, RUBBER_BAND) > limit * 0.99);
        // Tanda ikut arah tarikan.
        assert!(rubber_band_offset(-100.0, VIEWPORT, RUBBER_BAND) < 0.0);
        // Awalnya nyaris 1:1 — isi harus terasa menempel di jari.
        let kecil = rubber_band_offset(1.0, VIEWPORT, RUBBER_BAND);
        assert!(kecil > 0.99 && kecil < 1.0, "{kecil}");
    }

    /// Faktor per-langkah dan bentuk tertutupnya harus **rumus yang sama**:
    /// menjumlahkan langkah-langkah kecil harus mendarat di `f(x)`.
    #[test]
    fn integral_faktor_sama_dengan_bentuk_tertutup() {
        let langkah = 0.05f32;
        let mut pos = 0.0f32;
        let mut tarik = 0.0f32;
        while tarik < 200.0 {
            pos = apply_delta(pos, -langkah, 0.0, VIEWPORT, RUBBER_BAND);
            tarik += langkah;
        }
        let tutup = rubber_band_offset(-tarik, VIEWPORT, RUBBER_BAND);
        assert!(
            (pos - tutup).abs() < 0.5,
            "integral {pos} vs bentuk tertutup {tutup}"
        );
    }

    #[test]
    fn jarak_tarik_dan_simpangan_bolak_balik() {
        for raw in [0.0f32, 3.0, 40.0, 500.0] {
            let y = rubber_band_offset(raw, VIEWPORT, RUBBER_BAND);
            let kembali = rubber_band_raw(y, VIEWPORT, RUBBER_BAND);
            assert!((kembali - raw).abs() < 0.01, "{raw} -> {y} -> {kembali}");
        }
        // Di limit (dan seterusnya) tarikannya tak hingga — itu jawaban yang
        // benar, dan `apply_delta` menanganinya sebagai "berhenti di limit".
        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        assert!(rubber_band_raw(limit, VIEWPORT, RUBBER_BAND).is_infinite());
        assert!(rubber_band_raw(-3.0, VIEWPORT, RUBBER_BAND) < 0.0);
    }

    #[test]
    fn redaman_tidak_bergantung_ukuran_langkah() {
        // Satu langkah 100 poin harus mendarat di tempat yang sama dengan
        // seratus langkah 1 poin. Inilah yang gagal kalau faktor per-langkah
        // dipakai apa adanya untuk selisih sebesar event guliran.
        let max = 600.0;
        let sekali = apply_delta(600.0, 100.0, max, VIEWPORT, RUBBER_BAND);
        let mut bertahap = 600.0;
        for _ in 0..100 {
            bertahap = apply_delta(bertahap, 1.0, max, VIEWPORT, RUBBER_BAND);
        }
        assert!(
            (sekali - bertahap).abs() < 0.01,
            "sekali {sekali} vs bertahap {bertahap}"
        );
    }

    #[test]
    fn di_dalam_batas_bergerak_satu_banding_satu() {
        let max = 600.0;
        assert_eq!(apply_delta(0.0, 120.0, max, VIEWPORT, RUBBER_BAND), 120.0);
        assert_eq!(apply_delta(120.0, -50.0, max, VIEWPORT, RUBBER_BAND), 70.0);
        // Tepat sampai tepi tanpa redaman.
        assert_eq!(apply_delta(590.0, 10.0, max, VIEWPORT, RUBBER_BAND), 600.0);
    }

    #[test]
    fn melewati_tepi_langsung_teredam() {
        let max = 600.0;
        let baru = apply_delta(590.0, 60.0, max, VIEWPORT, RUBBER_BAND);
        assert!(baru > 600.0, "harus melar melewati tepi: {baru}");
        assert!(
            baru < 650.0,
            "50 poin sisanya harus teredam, bukan bergerak penuh: {baru}"
        );
    }

    #[test]
    fn kembali_dari_simpangan_tidak_pernah_diredam() {
        let max = 600.0;
        // Sedang melar 40 poin di atas; tarikan 40 poin harus mendarat pas di 0.
        assert_eq!(apply_delta(-40.0, 40.0, max, VIEWPORT, RUBBER_BAND), 0.0);
        // Lebih dari itu: sisanya masuk ke wilayah normal, tetap 1:1.
        assert_eq!(apply_delta(-40.0, 60.0, max, VIEWPORT, RUBBER_BAND), 20.0);
        // Simetris di ujung bawah.
        assert_eq!(apply_delta(640.0, -40.0, max, VIEWPORT, RUBBER_BAND), 600.0);
    }

    #[test]
    fn tarikan_gila_tetap_berhenti_di_limit() {
        let max = 600.0;
        let limit = overscroll_limit(VIEWPORT, RUBBER_BAND);
        let atas = apply_delta(0.0, -1.0e9, max, VIEWPORT, RUBBER_BAND);
        assert!(atas >= -limit && atas <= 0.0, "{atas}");
        let bawah = apply_delta(max, 1.0e9, max, VIEWPORT, RUBBER_BAND);
        assert!(bawah <= max + limit, "{bawah}");
        assert!(apply_delta(0.0, f32::NAN, max, VIEWPORT, RUBBER_BAND) == 0.0);
    }

    #[test]
    fn thumb_tidak_ada_saat_isi_muat() {
        assert!(thumb(400.0, 400.0, 0.0, 44.0).is_none());
        assert!(thumb(400.0, 200.0, 0.0, 44.0).is_none());
        assert!(thumb(400.0, 401.0, 0.0, 44.0).is_some());
    }

    #[test]
    fn panjang_thumb_sebanding_porsi_terlihat() {
        let t = thumb(400.0, 800.0, 0.0, 10.0).expect("bisa digulir");
        assert!((t.length - 200.0).abs() < 1e-3, "{t:?}");
        assert_eq!(t.offset, 0.0);

        // Di ujung bawah thumb menempel ke tepi akhir.
        let t = thumb(400.0, 800.0, 400.0, 10.0).expect("bisa digulir");
        assert!((t.end() - 400.0).abs() < 1e-3, "{t:?}");

        // Di tengah, tepat di tengah jalurnya.
        let t = thumb(400.0, 800.0, 200.0, 10.0).expect("bisa digulir");
        assert!((t.offset - 100.0).abs() < 1e-3, "{t:?}");
    }

    #[test]
    fn thumb_tidak_pernah_lebih_pendek_dari_hit_target() {
        // Isi 100× viewport: proporsional akan menghasilkan 4 poin — tidak bisa
        // diseret siapa pun.
        let t = thumb(400.0, 40_000.0, 0.0, 44.0).expect("bisa digulir");
        assert!(t.length >= 44.0, "{t:?}");
        // Viewport yang lebih pendek dari hit target tetap masuk akal.
        let t = thumb(30.0, 300.0, 0.0, 44.0).expect("bisa digulir");
        assert!(t.length <= 30.0, "{t:?}");
    }

    #[test]
    fn thumb_menyusut_saat_isi_melar() {
        let normal = thumb(400.0, 800.0, 0.0, 10.0).expect("bisa digulir");
        let melar = thumb(400.0, 800.0, -60.0, 10.0).expect("bisa digulir");
        assert!(melar.length < normal.length, "{melar:?} vs {normal:?}");
        assert_eq!(melar.offset, 0.0, "menempel di tepi yang dilewati");

        let bawah = thumb(400.0, 800.0, 460.0, 10.0).expect("bisa digulir");
        assert!(bawah.length < normal.length);
        assert!((bawah.end() - 400.0).abs() < 1e-3, "{bawah:?}");
        // Tidak pernah menyusut di bawah hit target.
        let ekstrem = thumb(400.0, 800.0, -1000.0, 44.0).expect("bisa digulir");
        assert!(ekstrem.length >= 44.0);
    }

    #[test]
    fn thumb_dan_kebalikannya_bolak_balik() {
        for offset in [0.0f32, 37.0, 200.0, 399.5, 400.0] {
            let t = thumb(400.0, 800.0, offset, 44.0).expect("bisa digulir");
            let kembali = scroll_for_thumb(400.0, 800.0, t.offset, 44.0);
            assert!(
                (kembali - offset).abs() < 0.01,
                "{offset} -> {t:?} -> {kembali}"
            );
        }
        // Seretan di luar jalur dijepit, bukan menghasilkan posisi liar.
        assert_eq!(scroll_for_thumb(400.0, 800.0, -100.0, 44.0), 0.0);
        assert_eq!(scroll_for_thumb(400.0, 800.0, 10_000.0, 44.0), 400.0);
        assert_eq!(scroll_for_thumb(400.0, 300.0, 50.0, 44.0), 0.0);
    }

    #[test]
    fn satu_halaman_menyisakan_satu_baris_tumpang_tindih() {
        assert_eq!(page_step(400.0, 20.0), 380.0);
        // Viewport mungil tidak boleh menghasilkan langkah nol/negatif.
        assert_eq!(page_step(30.0, 20.0), 15.0);
        assert!(page_step(0.0, 20.0) >= 0.0);
    }

    #[test]
    fn scroll_to_reveal_diam_bila_sudah_terlihat() {
        // Baris di tengah viewport: tidak ada alasan bergerak.
        assert_eq!(scroll_to_reveal(100.0, 400.0, 200.0, 40.0, 8.0), 100.0);
        // Di atas layar → naik sampai tepi atas + padding.
        assert_eq!(scroll_to_reveal(300.0, 400.0, 100.0, 40.0, 8.0), 92.0);
        // Di bawah layar → turun sampai ujungnya masuk + padding.
        assert_eq!(scroll_to_reveal(0.0, 400.0, 500.0, 40.0, 8.0), 148.0);
        // Isi lebih tinggi dari viewport: awalnya yang diprioritaskan.
        assert_eq!(scroll_to_reveal(0.0, 100.0, 50.0, 400.0, 0.0), 50.0);
    }

    #[test]
    fn kecepatan_dari_dua_sampel() {
        assert_eq!(velocity_from(30.0, Duration::from_millis(10)), 3000.0);
        assert_eq!(velocity_from(30.0, Duration::ZERO), 0.0);
        assert_eq!(velocity_from(f32::NAN, Duration::from_millis(10)), 0.0);
    }
}
