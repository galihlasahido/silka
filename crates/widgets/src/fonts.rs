//! [`Fonts`] — pegangan bersama ke mesin teks aplikasi.
//!
//! Memindai font sistem mahal dan glyph atlas **harus** dibagi (kalau tidak,
//! glyph yang sama dirasterisasi dua kali dan tekstur atlas digandakan), jadi
//! satu [`TextEngine`] hidup selama aplikasi berjalan dan dipakai bergantian
//! oleh dua pihak di UI thread yang sama:
//!
//! 1. **saat membangun view** — [`crate::text`] mengukur dan merasterisasi;
//! 2. **saat menggambar** — backend mengunggah bagian atlas yang berubah lewat
//!    [`rustui_paint::GlyphSource`].
//!
//! Keduanya tidak pernah berjalan bersamaan, jadi `Rc<RefCell<…>>` sudah cukup
//! dan tidak ada biaya sinkronisasi (REKOMENDASI §3.3).
//!
//! ```
//! use rustui_widgets::Fonts;
//!
//! // `bundled_only` = tanpa font sistem: cepat dan deterministik untuk test.
//! let fonts = Fonts::bundled_only();
//! fonts.set_scale_factor(2.0);
//! assert_eq!(fonts.scale_factor(), 2.0);
//! ```

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rustui_text::TextEngine;

/// Pegangan ber-`Clone` ke satu [`TextEngine`] milik aplikasi.
#[derive(Clone)]
pub struct Fonts(Rc<RefCell<TextEngine>>);

impl Fonts {
    /// Mesin dengan font bundel + fallback sistem — pilihan untuk aplikasi.
    pub fn new() -> Self {
        Self::from_engine(TextEngine::new())
    }

    /// Mesin tanpa font sistem: **deterministik**, untuk unit test dan CI
    /// (§9.5).
    pub fn bundled_only() -> Self {
        Self::from_engine(TextEngine::bundled_only())
    }

    /// Bungkus mesin yang sudah ada.
    pub fn from_engine(engine: TextEngine) -> Self {
        Self(Rc::new(RefCell::new(engine)))
    }

    /// Pegangan mentahnya — inilah yang diserahkan ke
    /// `WindowConfig::glyphs(…)` supaya atlas yang diisi saat membangun view
    /// **persis** atlas yang dibaca backend.
    pub fn shared(&self) -> Rc<RefCell<TextEngine>> {
        self.0.clone()
    }

    /// Pinjam mesinnya sebentar.
    ///
    /// Panik bila dipanggil saat mesin sedang dipinjam pihak lain — sengaja:
    /// itu berarti ada dua pemakai yang berjalan bersamaan, dan janji "tidak
    /// pernah bersamaan" sudah dilanggar.
    pub fn with<R>(&self, f: impl FnOnce(&mut TextEngine) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    /// Scale factor yang sedang dipakai untuk rasterisasi.
    pub fn scale_factor(&self) -> f32 {
        self.0.borrow().scale_factor()
    }

    /// Setel scale factor layar (§3.3). Ukuran logis tidak ikut berubah.
    pub fn set_scale_factor(&self, scale_factor: f32) {
        self.0.borrow_mut().set_scale_factor(scale_factor);
    }

    /// Benar bila dua pegangan menunjuk mesin yang sama.
    pub fn ptr_eq(&self, other: &Fonts) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Fonts {
    /// Identitas, bukan isi: mesin teks tidak punya kesamaan struktural yang
    /// bermakna, dan yang penting bagi diffing memang "mesin yang sama".
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
    }
}

impl fmt::Debug for Fonts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fonts")
            .field("scale_factor", &self.scale_factor())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klon_menunjuk_mesin_yang_sama() {
        let a = Fonts::bundled_only();
        let b = a.clone();
        b.set_scale_factor(3.0);
        assert_eq!(a.scale_factor(), 3.0, "atlas harus dibagi, bukan disalin");
        assert_eq!(a, b);
        assert_ne!(a, Fonts::bundled_only());
    }

    #[test]
    fn scale_factor_tidak_masuk_akal_ditolak() {
        let f = Fonts::bundled_only();
        f.set_scale_factor(0.0);
        assert_eq!(f.scale_factor(), 1.0);
    }
}
