//! Palet **mentah** — angka warna apa adanya, belum punya arti.
//!
//! Lapisan ini sengaja dipisah dari token semantik ([`crate::ColorTokens`]):
//! di sinilah satu-satunya tempat literal warna boleh hidup. Preset membaca
//! palet ini dan memberinya peran (`surface`, `accent`, …); widget tidak pernah
//! menyentuh modul ini sama sekali (REKOMENDASI §2.6, §2.7).
//!
//! Dua sumber angka:
//!
//! - [`tailwind`] — ramp 11 langkah 50–950, disalin apa adanya dari palet
//!   Tailwind. Inilah "tampilan Tailwind" yang orang kenal: bukan CSS-nya,
//!   melainkan angkanya (§2.6).
//! - [`hig`] — warna sistem Apple (systemBlue, label, separator, fill) beserta
//!   pasangan light/dark-nya.
//!
//! Nilainya disimpan sebagai literal hex `u32`, bukan [`Color`], supaya bisa
//! jadi `const` tanpa aritmetika float di `const fn` (batas `rust-version`
//! workspace).

use rustui_paint::Color;

/// Satu langkah pada ramp 50–950.
///
/// Urutannya sama dengan yang tertulis di palet: [`Step::S50`] paling terang,
/// [`Step::S950`] paling gelap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    /// Langkah 50 — nyaris putih.
    S50,
    /// Langkah 100.
    S100,
    /// Langkah 200.
    S200,
    /// Langkah 300.
    S300,
    /// Langkah 400.
    S400,
    /// Langkah 500 — titik tengah ramp.
    S500,
    /// Langkah 600.
    S600,
    /// Langkah 700.
    S700,
    /// Langkah 800.
    S800,
    /// Langkah 900.
    S900,
    /// Langkah 950 — nyaris hitam.
    S950,
}

impl Step {
    /// Semua langkah, dari paling terang ke paling gelap.
    pub const ALL: [Step; 11] = [
        Step::S50,
        Step::S100,
        Step::S200,
        Step::S300,
        Step::S400,
        Step::S500,
        Step::S600,
        Step::S700,
        Step::S800,
        Step::S900,
        Step::S950,
    ];

    /// Angka langkah seperti yang ditulis orang (`slate-500` → `500`).
    pub const fn value(self) -> u16 {
        match self {
            Step::S50 => 50,
            Step::S100 => 100,
            Step::S200 => 200,
            Step::S300 => 300,
            Step::S400 => 400,
            Step::S500 => 500,
            Step::S600 => 600,
            Step::S700 => 700,
            Step::S800 => 800,
            Step::S900 => 900,
            Step::S950 => 950,
        }
    }

    /// Posisi langkah di dalam larik [`Ramp`].
    pub const fn index(self) -> usize {
        match self {
            Step::S50 => 0,
            Step::S100 => 1,
            Step::S200 => 2,
            Step::S300 => 3,
            Step::S400 => 4,
            Step::S500 => 5,
            Step::S600 => 6,
            Step::S700 => 7,
            Step::S800 => 8,
            Step::S900 => 9,
            Step::S950 => 10,
        }
    }
}

/// Ramp warna 11 langkah (50–950).
///
/// ```
/// use rustui_theme::palette::{tailwind, Step};
///
/// // `bg-slate-800` di web = warna ini, tanpa CSS apa pun.
/// assert_eq!(tailwind::SLATE.hex(Step::S800), 0x1E293B);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ramp([u32; 11]);

impl Ramp {
    /// Ramp dari 11 literal hex, urut 50 → 950.
    pub const fn new(hex: [u32; 11]) -> Self {
        Self(hex)
    }

    /// Literal hex satu langkah.
    pub const fn hex(self, step: Step) -> u32 {
        self.0[step.index()]
    }

    /// Warna satu langkah.
    pub fn get(self, step: Step) -> Color {
        Color::hex(self.hex(step))
    }

    /// Seluruh ramp sebagai warna, urut 50 → 950.
    pub fn shades(self) -> [Color; 11] {
        let mut out = [Color::TRANSPARENT; 11];
        for (i, step) in Step::ALL.iter().enumerate() {
            out[i] = self.get(*step);
        }
        out
    }
}

/// Ramp Tailwind yang dipakai preset `Tailwind/shadcn` (§2.7).
///
/// `slate` adalah warna netral shadcn/ui; `blue` adalah aksennya. Tiga ramp
/// lain melayani token status (destructive/success/warning) supaya preset ini
/// tidak perlu meminjam warna dari HIG.
pub mod tailwind {
    use super::Ramp;

    /// Netral berbias biru — dasar seluruh permukaan dan teks preset ini.
    pub const SLATE: Ramp = Ramp::new([
        0xF8FAFC, 0xF1F5F9, 0xE2E8F0, 0xCBD5E1, 0x94A3B8, 0x64748B, 0x475569, 0x334155, 0x1E293B,
        0x0F172A, 0x020617,
    ]);

    /// Aksen utama shadcn/ui.
    pub const BLUE: Ramp = Ramp::new([
        0xEFF6FF, 0xDBEAFE, 0xBFDBFE, 0x93C5FD, 0x60A5FA, 0x3B82F6, 0x2563EB, 0x1D4ED8, 0x1E40AF,
        0x1E3A8A, 0x172554,
    ]);

    /// Aksi destruktif.
    pub const RED: Ramp = Ramp::new([
        0xFEF2F2, 0xFEE2E2, 0xFECACA, 0xFCA5A5, 0xF87171, 0xEF4444, 0xDC2626, 0xB91C1C, 0x991B1B,
        0x7F1D1D, 0x450A0A,
    ]);

    /// Status berhasil.
    pub const EMERALD: Ramp = Ramp::new([
        0xECFDF5, 0xD1FAE5, 0xA7F3D0, 0x6EE7B7, 0x34D399, 0x10B981, 0x059669, 0x047857, 0x065F46,
        0x064E3B, 0x022C22,
    ]);

    /// Status peringatan.
    pub const AMBER: Ramp = Ramp::new([
        0xFFFBEB, 0xFEF3C7, 0xFDE68A, 0xFCD34D, 0xFBBF24, 0xF59E0B, 0xD97706, 0xB45309, 0x92400E,
        0x78350F, 0x451A03,
    ]);
}

/// Warna sistem Apple (HIG) untuk preset `Cupertino`.
///
/// Apple menerbitkan **pasangan** light/dark untuk setiap warna — bukan satu
/// warna yang digelapkan otomatis. Karena itu setiap konstanta di sini punya
/// varian `_LIGHT` dan `_DARK`, dan preset memilih berdasarkan
/// [`crate::Appearance`].
///
/// Warna label/separator/fill di HIG **semi-transparan** (mereka menyatu
/// dengan material di belakangnya). Alpha-nya disimpan terpisah sebagai
/// konstanta `*_ALPHA` supaya bisa dipasang lewat
/// [`rustui_paint::Color::with_alpha`].
pub mod hig {
    /// systemBlue — warna aksen default macOS/iOS (light).
    pub const SYSTEM_BLUE_LIGHT: u32 = 0x007AFF;
    /// systemBlue (dark).
    pub const SYSTEM_BLUE_DARK: u32 = 0x0A84FF;
    /// systemBlue satu tingkat lebih pekat — dipakai untuk hover di light.
    pub const SYSTEM_BLUE_PRESSED_LIGHT: u32 = 0x0069DB;
    /// systemBlue satu tingkat lebih terang — hover di dark.
    pub const SYSTEM_BLUE_PRESSED_DARK: u32 = 0x409CFF;

    /// systemRed (light).
    pub const SYSTEM_RED_LIGHT: u32 = 0xFF3B30;
    /// systemRed (dark).
    pub const SYSTEM_RED_DARK: u32 = 0xFF453A;
    /// systemGreen (light).
    pub const SYSTEM_GREEN_LIGHT: u32 = 0x34C759;
    /// systemGreen (dark).
    pub const SYSTEM_GREEN_DARK: u32 = 0x30D158;
    /// systemOrange (light).
    pub const SYSTEM_ORANGE_LIGHT: u32 = 0xFF9500;
    /// systemOrange (dark).
    pub const SYSTEM_ORANGE_DARK: u32 = 0xFF9F0A;

    /// systemGroupedBackground (light) — latar window bergaya Settings.
    pub const GROUPED_BACKGROUND_LIGHT: u32 = 0xF2F2F7;
    /// Latar window (dark).
    pub const GROUPED_BACKGROUND_DARK: u32 = 0x1C1C1E;
    /// secondarySystemGroupedBackground (light) — permukaan kartu.
    pub const SURFACE_LIGHT: u32 = 0xFFFFFF;
    /// Permukaan kartu (dark).
    pub const SURFACE_DARK: u32 = 0x2C2C2E;
    /// tertiarySystemGroupedBackground (dark) — permukaan terangkat.
    pub const SURFACE_ELEVATED_DARK: u32 = 0x3A3A3C;
    /// Permukaan "cekung" (dark), mis. dasar scroll area.
    pub const SURFACE_SUNKEN_DARK: u32 = 0x141416;
    /// Permukaan "cekung" (light).
    pub const SURFACE_SUNKEN_LIGHT: u32 = 0xE9E9EE;

    /// Warna dasar label di light — hitam murni, alpha yang membedakan tingkat.
    pub const LABEL_LIGHT: u32 = 0x000000;
    /// Warna dasar label di dark.
    pub const LABEL_DARK: u32 = 0xFFFFFF;
    /// Warna dasar label sekunder/tersier di light (`#3C3C43`).
    pub const LABEL_TINT_LIGHT: u32 = 0x3C3C43;
    /// Warna dasar label sekunder/tersier di dark (`#EBEBF5`).
    pub const LABEL_TINT_DARK: u32 = 0xEBEBF5;

    /// Alpha secondaryLabel.
    pub const SECONDARY_LABEL_ALPHA: f32 = 0.60;
    /// Alpha tertiaryLabel.
    pub const TERTIARY_LABEL_ALPHA: f32 = 0.30;
    /// Alpha quaternaryLabel — teks non-aktif.
    pub const QUATERNARY_LABEL_ALPHA: f32 = 0.18;

    /// Warna dasar separator (light).
    pub const SEPARATOR_LIGHT: u32 = 0x3C3C43;
    /// Alpha separator (light).
    pub const SEPARATOR_ALPHA_LIGHT: f32 = 0.29;
    /// Warna dasar separator (dark).
    pub const SEPARATOR_DARK: u32 = 0x545458;
    /// Alpha separator (dark).
    pub const SEPARATOR_ALPHA_DARK: f32 = 0.65;

    /// systemFill — latar sementara kontrol (light).
    pub const FILL_LIGHT: u32 = 0x787880;
    /// systemFill (dark).
    pub const FILL_DARK: u32 = 0x7C7C80;
    /// Alpha quaternarySystemFill — dipakai hover permukaan.
    pub const FILL_HOVER_ALPHA: f32 = 0.12;
    /// Alpha tertiarySystemFill — dipakai state pressed.
    pub const FILL_PRESSED_ALPHA: f32 = 0.20;

    /// Scrim modal (dim di belakang sheet/dialog), light.
    pub const SCRIM_ALPHA_LIGHT: f32 = 0.20;
    /// Scrim modal, dark.
    pub const SCRIM_ALPHA_DARK: f32 = 0.45;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminansi(c: Color) -> f32 {
        let [r, g, b, _] = c.to_linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    #[test]
    fn langkah_urut_dan_indeksnya_rapat() {
        for (i, step) in Step::ALL.iter().enumerate() {
            assert_eq!(step.index(), i);
        }
        let nilai: Vec<u16> = Step::ALL.iter().map(|s| s.value()).collect();
        assert_eq!(nilai[0], 50);
        assert_eq!(nilai[10], 950);
        assert!(nilai.windows(2).all(|w| w[0] < w[1]), "{nilai:?}");
    }

    #[test]
    fn setiap_ramp_makin_gelap_dari_50_ke_950() {
        for (nama, ramp) in [
            ("slate", tailwind::SLATE),
            ("blue", tailwind::BLUE),
            ("red", tailwind::RED),
            ("emerald", tailwind::EMERALD),
            ("amber", tailwind::AMBER),
        ] {
            let l: Vec<f32> = ramp.shades().iter().map(|c| luminansi(*c)).collect();
            assert!(
                l.windows(2).all(|w| w[0] > w[1]),
                "{nama} tidak monoton: {l:?}"
            );
        }
    }

    #[test]
    fn nilai_ramp_sama_persis_dengan_palet_tailwind() {
        // Angka-angka inilah yang membuat "tampilan Tailwind" — kalau meleset,
        // preset kedua kehilangan alasan keberadaannya (§2.6).
        assert_eq!(tailwind::SLATE.hex(Step::S50), 0xF8FAFC);
        assert_eq!(tailwind::SLATE.hex(Step::S500), 0x64748B);
        assert_eq!(tailwind::SLATE.hex(Step::S950), 0x020617);
        assert_eq!(tailwind::BLUE.hex(Step::S500), 0x3B82F6);
        assert_eq!(tailwind::BLUE.hex(Step::S600), 0x2563EB);
        assert_eq!(tailwind::RED.hex(Step::S600), 0xDC2626);
    }

    #[test]
    fn get_dan_hex_menghasilkan_warna_yang_sama() {
        let c = tailwind::BLUE.get(Step::S600);
        assert_eq!(c, Color::hex(0x2563EB));
        assert_eq!(tailwind::BLUE.shades()[Step::S600.index()], c);
    }

    #[test]
    fn hig_punya_pasangan_light_dan_dark_yang_berbeda() {
        for (nama, terang, gelap) in [
            ("blue", hig::SYSTEM_BLUE_LIGHT, hig::SYSTEM_BLUE_DARK),
            ("red", hig::SYSTEM_RED_LIGHT, hig::SYSTEM_RED_DARK),
            ("green", hig::SYSTEM_GREEN_LIGHT, hig::SYSTEM_GREEN_DARK),
            ("orange", hig::SYSTEM_ORANGE_LIGHT, hig::SYSTEM_ORANGE_DARK),
        ] {
            assert_ne!(terang, gelap, "{nama}: dark mode bukan sekadar digelapkan");
        }
    }

    #[test]
    fn alpha_label_hig_menurun_per_tingkat() {
        let a = [
            hig::SECONDARY_LABEL_ALPHA,
            hig::TERTIARY_LABEL_ALPHA,
            hig::QUATERNARY_LABEL_ALPHA,
        ];
        assert!(a.windows(2).all(|w| w[0] > w[1]), "{a:?}");
        assert!(a.iter().all(|x| *x > 0.0), "{a:?}");
    }
}
