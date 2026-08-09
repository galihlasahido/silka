//! Skala font semantik + gaya teks per token.
//!
//! Widget menyebut **peran** (`Body`, `Headline`, `Caption1`), bukan angka —
//! sama seperti warna. Yang berbeda antar preset bukan hanya ukurannya:
//!
//! | | Cupertino | Tailwind/shadcn |
//! |---|---|---|
//! | Ukuran | skala teks HIG (10 → 26pt) | skala Tailwind (12 → 30px) |
//! | Tinggi baris | pasangan HIG (13/16/20/26/32) | pasangan Tailwind (16/20/28/32/36) |
//! | Optical size | **ya** — sumbu `opsz` Inter v4 diikat ke ukuran | tidak |
//! | Tracking | tabel ala SF: longgar di kecil, rapat di besar | 0, kecuali judul besar |
//!
//! Crate ini sengaja **tidak** bergantung pada `silka-text`: token adalah
//! nilai murni, dan sebuah crate token tidak boleh menyeret font shaper ke
//! dalam pohon dependensi. Pemetaan ke `silka_text::TextStyle` terjadi di
//! lapisan widget:
//!
//! ```ignore
//! let ts = theme.font(FontToken::Headline);
//! TextStyle::new()
//!     .size(ts.size)
//!     .weight(FontWeight(ts.weight))
//!     .line_height(ts.line_height)
//!     .tracking(ts.tracking)
//! ```

/// Berat font pada skala CSS/OpenType — nilai yang dipakai token.
///
/// Inter yang dibundel adalah variable font, jadi angka apa pun 1–1000 sah;
/// konstanta di sini hanya nama untuk yang lazim.
pub mod weight {
    /// 400 — teks body.
    pub const REGULAR: u16 = 400;
    /// 500 — label kontrol (tombol, tab).
    pub const MEDIUM: u16 = 500;
    /// 600 — judul ala HIG.
    pub const SEMIBOLD: u16 = 600;
    /// 700 — judul besar.
    pub const BOLD: u16 = 700;
}

/// Rentang sumbu `opsz` (optical size) pada Inter v4.
///
/// Di luar rentang ini font tidak punya master, jadi nilai harus di-clamp —
/// bukan diekstrapolasi.
pub const INTER_OPSZ_RANGE: (f32, f32) = (14.0, 32.0);

/// Gaya teks satu token: nilai murni, siap dipetakan ke `TextStyle`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeStyle {
    /// Ukuran font dalam poin logis.
    pub size: f32,
    /// Tinggi baris sebagai **kelipatan** ukuran font.
    pub line_height: f32,
    /// Berat font (lihat [`weight`]).
    pub weight: u16,
    /// Tracking dalam em — negatif merapatkan, ala SF pada ukuran besar.
    pub tracking: f32,
    /// Nilai sumbu `opsz` yang diminta, bila preset memakai optical sizing.
    ///
    /// `None` berarti "biarkan font memakai master default" — itulah perilaku
    /// preset Tailwind, yang memang tidak meniru optical sizing SF.
    pub optical_size: Option<f32>,
}

impl TypeStyle {
    /// Gaya dari ukuran dan tinggi baris **dalam poin** (bukan kelipatan).
    ///
    /// Bentuk inilah yang dipakai tabel HIG maupun Tailwind: keduanya menulis
    /// "13/16", bukan "13 × 1,23".
    pub fn new(size: f32, line_height_px: f32) -> Self {
        let size = size.max(1.0);
        Self {
            size,
            line_height: (line_height_px / size).max(0.0),
            weight: weight::REGULAR,
            tracking: 0.0,
            optical_size: None,
        }
    }

    /// Setel berat.
    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight.clamp(1, 1000);
        self
    }

    /// Setel tracking dalam em.
    pub fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    /// Hidupkan optical sizing: sumbu `opsz` diikat ke ukuran font, di-clamp ke
    /// rentang yang benar-benar ada di Inter v4.
    pub fn optical(mut self) -> Self {
        let (min, max) = INTER_OPSZ_RANGE;
        self.optical_size = Some(self.size.clamp(min, max));
        self
    }

    /// Tinggi baris dalam poin logis.
    pub fn line_height_px(self) -> f32 {
        (self.size * self.line_height).max(1.0)
    }
}

/// Tracking ala SF: longgar di ukuran kecil, rapat di ukuran besar.
///
/// Ini yang membuat teks "terasa Apple" jauh sebelum orang sadar kenapa —
/// SF Pro punya tabel tracking per ukuran, dan Inter (yang menggantikannya
/// karena SF tidak boleh di-ship) perlu ditiru manual. Nilainya em, hasil
/// interpolasi linear di antara titik-titik tabel; di luar tabel ia mendatar.
pub fn optical_tracking(size: f32) -> f32 {
    /// (ukuran poin, tracking em) — disarikan dari tabel tracking SF Pro.
    const TABEL: [(f32, f32); 11] = [
        (6.0, 0.041),
        (8.0, 0.025),
        (10.0, 0.012),
        (11.0, 0.006),
        (12.0, 0.0),
        (13.0, -0.006),
        (14.0, -0.011),
        (16.0, -0.020),
        (17.0, -0.024),
        (24.0, -0.019),
        (48.0, -0.022),
    ];

    if size <= TABEL[0].0 {
        return TABEL[0].1;
    }
    for w in TABEL.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if size <= x1 {
            let t = (size - x0) / (x1 - x0);
            return y0 + (y1 - y0) * t;
        }
    }
    TABEL[TABEL.len() - 1].1
}

/// Nama token tipografi.
///
/// Kosakatanya mengikuti HIG karena di sanalah perannya paling eksplisit;
/// preset Tailwind memetakannya ke `text-xs`…`text-3xl`. Widget menulis
/// `FontToken::Body` sekali dan benar di kedua preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontToken {
    /// Teks terkecil (legenda chart, footer tabel).
    Caption2,
    /// Keterangan kecil (label ikon, badge).
    Caption1,
    /// Catatan kaki.
    Footnote,
    /// Sub-judul baris (keterangan di bawah judul list).
    Subheadline,
    /// Teks pendamping (label kontrol sekunder).
    Callout,
    /// Teks body — ukuran default seluruh UI.
    Body,
    /// Body dengan penekanan (judul baris list, label tombol).
    Headline,
    /// Judul kecil.
    Title3,
    /// Judul sedang.
    Title2,
    /// Judul besar.
    Title1,
    /// Judul halaman.
    LargeTitle,
}

impl FontToken {
    /// Semua token, dari terkecil ke terbesar.
    pub const ALL: [FontToken; 11] = [
        FontToken::Caption2,
        FontToken::Caption1,
        FontToken::Footnote,
        FontToken::Subheadline,
        FontToken::Callout,
        FontToken::Body,
        FontToken::Headline,
        FontToken::Title3,
        FontToken::Title2,
        FontToken::Title1,
        FontToken::LargeTitle,
    ];

    /// Nama token untuk gallery/debug.
    pub const fn name(self) -> &'static str {
        match self {
            FontToken::Caption2 => "caption2",
            FontToken::Caption1 => "caption1",
            FontToken::Footnote => "footnote",
            FontToken::Subheadline => "subheadline",
            FontToken::Callout => "callout",
            FontToken::Body => "body",
            FontToken::Headline => "headline",
            FontToken::Title3 => "title3",
            FontToken::Title2 => "title2",
            FontToken::Title1 => "title1",
            FontToken::LargeTitle => "large_title",
        }
    }
}

/// Token tipografi lengkap satu preset.
///
/// `body_size` dan `body_line_height` adalah bentuk pendek dari
/// `body` — dipertahankan karena itulah yang paling sering dipakai widget, dan
/// keduanya **diturunkan**, tidak diisi terpisah (lihat
/// [`TypographyTokens::new`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypographyTokens {
    /// Ukuran teks body dalam poin logis.
    pub body_size: f32,
    /// Tinggi baris body relatif terhadap ukuran font.
    pub body_line_height: f32,
    /// Preset ini mengikat sumbu `opsz` ke ukuran font.
    pub optical_sizing: bool,
    /// [`FontToken::Caption2`].
    pub caption2: TypeStyle,
    /// [`FontToken::Caption1`].
    pub caption1: TypeStyle,
    /// [`FontToken::Footnote`].
    pub footnote: TypeStyle,
    /// [`FontToken::Subheadline`].
    pub subheadline: TypeStyle,
    /// [`FontToken::Callout`].
    pub callout: TypeStyle,
    /// [`FontToken::Body`].
    pub body: TypeStyle,
    /// [`FontToken::Headline`].
    pub headline: TypeStyle,
    /// [`FontToken::Title3`].
    pub title3: TypeStyle,
    /// [`FontToken::Title2`].
    pub title2: TypeStyle,
    /// [`FontToken::Title1`].
    pub title1: TypeStyle,
    /// [`FontToken::LargeTitle`].
    pub large_title: TypeStyle,
}

impl TypographyTokens {
    /// Susun skala dari 11 gaya, urut sesuai [`FontToken::ALL`].
    ///
    /// `body_size`/`body_line_height` diturunkan dari gaya `Body` supaya tidak
    /// mungkin melenceng dari skalanya sendiri.
    pub fn new(optical_sizing: bool, styles: [TypeStyle; 11]) -> Self {
        let body = styles[FontToken::Body as usize];
        Self {
            body_size: body.size,
            body_line_height: body.line_height,
            optical_sizing,
            caption2: styles[0],
            caption1: styles[1],
            footnote: styles[2],
            subheadline: styles[3],
            callout: styles[4],
            body,
            headline: styles[6],
            title3: styles[7],
            title2: styles[8],
            title1: styles[9],
            large_title: styles[10],
        }
    }

    /// Gaya satu token.
    pub fn get(&self, token: FontToken) -> TypeStyle {
        match token {
            FontToken::Caption2 => self.caption2,
            FontToken::Caption1 => self.caption1,
            FontToken::Footnote => self.footnote,
            FontToken::Subheadline => self.subheadline,
            FontToken::Callout => self.callout,
            FontToken::Body => self.body,
            FontToken::Headline => self.headline,
            FontToken::Title3 => self.title3,
            FontToken::Title2 => self.title2,
            FontToken::Title1 => self.title1,
            FontToken::LargeTitle => self.large_title,
        }
    }

    /// Seluruh skala urut kecil → besar, berpasangan dengan tokennya.
    pub fn scale(&self) -> [(FontToken, TypeStyle); 11] {
        let mut out = [(FontToken::Body, self.body); 11];
        for (i, token) in FontToken::ALL.iter().enumerate() {
            out[i] = (*token, self.get(*token));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};

    #[test]
    fn tinggi_baris_ditulis_dalam_poin_disimpan_sebagai_kelipatan() {
        let s = TypeStyle::new(13.0, 16.0);
        assert!((s.line_height - 16.0 / 13.0).abs() < 1e-6);
        assert!((s.line_height_px() - 16.0).abs() < 1e-4);
    }

    #[test]
    fn nilai_tak_masuk_akal_dijinakkan() {
        let s = TypeStyle::new(0.0, 0.0);
        assert!(s.size >= 1.0);
        assert!(s.line_height_px() >= 1.0);
        assert_eq!(TypeStyle::new(13.0, 16.0).weight(5_000).weight, 1_000);
    }

    #[test]
    fn optical_size_dibatasi_ke_rentang_inter() {
        let (min, max) = INTER_OPSZ_RANGE;
        assert_eq!(TypeStyle::new(10.0, 13.0).optical().optical_size, Some(min));
        assert_eq!(
            TypeStyle::new(96.0, 100.0).optical().optical_size,
            Some(max)
        );
        assert_eq!(
            TypeStyle::new(20.0, 24.0).optical().optical_size,
            Some(20.0)
        );
    }

    #[test]
    fn tracking_longgar_di_kecil_rapat_di_besar() {
        assert!(optical_tracking(9.0) > 0.0);
        assert!(optical_tracking(12.0).abs() < 1e-6);
        assert!(optical_tracking(17.0) < -0.02);
        assert!(optical_tracking(64.0) < 0.0);
        // Di luar tabel nilainya mendatar, tidak meledak.
        assert_eq!(optical_tracking(1.0), optical_tracking(6.0));
        assert_eq!(optical_tracking(200.0), optical_tracking(48.0));
        // Dan tetap dalam rentang yang masuk akal untuk teks UI.
        for i in 0..=200 {
            let t = optical_tracking(i as f32);
            assert!((-0.05..=0.05).contains(&t), "tracking {t} di ukuran {i}");
        }
    }

    #[test]
    fn skala_tidak_pernah_mengecil() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light).typography;
            let ukuran: Vec<f32> = t.scale().iter().map(|(_, s)| s.size).collect();
            assert!(
                ukuran.windows(2).all(|w| w[0] <= w[1]),
                "{preset:?}: {ukuran:?}"
            );
            let tinggi: Vec<f32> = t.scale().iter().map(|(_, s)| s.line_height_px()).collect();
            assert!(
                tinggi.windows(2).all(|w| w[0] <= w[1]),
                "{preset:?}: {tinggi:?}"
            );
        }
    }

    #[test]
    fn body_pendek_selalu_sama_dengan_token_body() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light).typography;
            assert_eq!(t.body_size, t.get(FontToken::Body).size, "{preset:?}");
            assert_eq!(
                t.body_line_height,
                t.get(FontToken::Body).line_height,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn headline_menekankan_lewat_berat_bukan_ukuran() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light).typography;
            let body = t.get(FontToken::Body);
            let headline = t.get(FontToken::Headline);
            assert_eq!(headline.size, body.size, "{preset:?}");
            assert!(headline.weight > body.weight, "{preset:?}");
        }
    }

    #[test]
    fn hanya_cupertino_yang_memakai_optical_sizing() {
        let cup = Theme::cupertino(Appearance::Light).typography;
        assert!(cup.optical_sizing);
        for (token, s) in cup.scale() {
            assert!(s.optical_size.is_some(), "{}", token.name());
        }

        let tw = Theme::tailwind(Appearance::Light).typography;
        assert!(!tw.optical_sizing);
        for (token, s) in tw.scale() {
            assert!(s.optical_size.is_none(), "{}", token.name());
        }
    }

    #[test]
    fn cupertino_merapatkan_judul_dan_melonggarkan_caption() {
        let t = Theme::cupertino(Appearance::Light).typography;
        assert!(t.get(FontToken::LargeTitle).tracking < 0.0);
        assert!(t.get(FontToken::Caption2).tracking > 0.0);
        assert!(t.get(FontToken::LargeTitle).tracking < t.get(FontToken::Body).tracking);
    }

    #[test]
    fn nama_token_unik_dan_urut() {
        let mut nama: Vec<&str> = FontToken::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(nama.len(), 11);
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
        // Urutan enum = urutan skala; `new()` mengandalkan itu.
        assert_eq!(FontToken::Body as usize, 5);
        for (i, token) in FontToken::ALL.iter().enumerate() {
            assert_eq!(*token as usize, i);
        }
    }
}
