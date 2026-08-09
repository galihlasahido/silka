//! Reduced-motion: setting aksesibilitas OS sebagai bagian kontrak animasi.

use super::spring::Spring;

/// Peran sebuah gerakan — menentukan apa yang terjadi saat reduced-motion aktif.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MotionRole {
    /// Gerakan yang **menjelaskan sesuatu**: sheet naik dari bawah, disclosure
    /// membuka, thumb toggle bergeser. Menghapusnya berarti menghapus
    /// informasi, jadi di bawah reduced-motion ia tetap bergerak — hanya
    /// pantulannya yang dibuang.
    #[default]
    Essential,
    /// Gerakan **dekoratif**: parallax, bounce hiasan, wiggle. Tidak membawa
    /// informasi apa pun, jadi di bawah reduced-motion ia dimatikan total.
    Decorative,
}

/// Preferensi gerakan pengguna, datang dari setting aksesibilitas OS.
///
/// macOS: "Reduce motion"; Windows: `PostAnimationsEnabled`; GNOME:
/// `gtk-enable-animations`. Lapisan platform yang membacanya; di sini ia cuma
/// dua keadaan, dan **setiap** nilai teranimasi wajib melewatinya
/// (KOMPONEN.md, definition of done).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Motion {
    /// Gerakan penuh seperti yang ditulis penulis widget.
    #[default]
    Full,
    /// Pengguna meminta gerakan dikurangi.
    ///
    /// Aturannya (INTEGRASI-NATIVE §"Reduced motion"): **matikan pantulan**,
    /// jangan matikan gerakan yang menjelaskan. Spring tetap berjalan tapi
    /// dijadikan critically damped, sehingga transisi tetap terbaca tanpa
    /// osilasi yang memicu vertigo. Gerakan [`MotionRole::Decorative`] hilang
    /// sepenuhnya.
    Reduced,
}

impl Motion {
    /// Bangun dari flag boolean milik platform.
    pub fn from_reduced(reduced: bool) -> Self {
        if reduced {
            Motion::Reduced
        } else {
            Motion::Full
        }
    }

    /// Benar bila pengguna meminta gerakan dikurangi.
    pub fn is_reduced(self) -> bool {
        matches!(self, Motion::Reduced)
    }

    /// Spring yang benar-benar dipakai di bawah preferensi ini.
    pub fn spring(self, spring: Spring) -> Spring {
        match self {
            Motion::Full => spring,
            Motion::Reduced => spring.without_bounce(),
        }
    }

    /// Benar bila gerakan dengan peran ini harus dihilangkan sama sekali.
    pub fn suppresses(self, role: MotionRole) -> bool {
        self.is_reduced() && role == MotionRole::Decorative
    }

    /// Nama pendek untuk log.
    pub const fn label(self) -> &'static str {
        match self {
            Motion::Full => "full",
            Motion::Reduced => "reduced",
        }
    }
}
