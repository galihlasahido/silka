//! Sumber font: **Inter yang dibundel** plus fallback sistem.
//!
//! Kenapa Inter (REKOMENDASI §3.6): SF Pro tidak boleh di-ship karena lisensi
//! Apple, sedangkan Inter open (SIL OFL 1.1), sangat dekat rasanya dengan SF,
//! dan versi 4 punya axis `opsz` (optical size). Yang dibundel di sini adalah
//! `InterVariable.ttf` — satu file variable font berisi seluruh rentang berat,
//! sehingga `weight(600)` bukan file kedua melainkan setelan axis.
//!
//! Fallback sistem tetap wajib: Inter tidak memuat CJK, Arab, maupun emoji.
//! Font sistem yang menutupinya (§3.3 "font fallback per platform"), dan
//! cosmic-text yang memilihnya per-script.

use std::sync::Arc;

use cosmic_text::{Family, FontSystem};

use crate::style::FontFamily;

/// Berkas font UI yang dibundel ke dalam binary.
///
/// Lisensi: SIL Open Font License 1.1 — lihat `assets/fonts/LICENSE-Inter.txt`.
pub const BUNDLED_UI_FONT: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");

/// Dari mana font diambil saat [`crate::TextEngine`] dibuat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontOptions {
    /// Muat font UI bundel (Inter). Hampir selalu benar.
    pub bundled_ui_font: bool,
    /// Muat font sistem sebagai fallback (CJK, Arab, emoji).
    ///
    /// Mematikannya membuat hasil **deterministik** — itulah yang dipakai unit
    /// test dan CI, karena mesin CI punya daftar font yang berbeda-beda.
    pub system_fonts: bool,
}

impl Default for FontOptions {
    fn default() -> Self {
        Self {
            bundled_ui_font: true,
            system_fonts: true,
        }
    }
}

impl FontOptions {
    /// Hanya font bundel — cepat dan deterministik, tanpa fallback.
    pub fn bundled_only() -> Self {
        Self {
            bundled_ui_font: true,
            system_fonts: false,
        }
    }
}

/// Hasil pemuatan font.
pub(crate) struct LoadedFonts {
    pub(crate) system: FontSystem,
    /// Nama keluarga font UI seperti yang dilaporkan berkasnya sendiri —
    /// dibaca dari font, bukan ditebak sebagai konstanta.
    pub(crate) ui_family: Option<String>,
}

pub(crate) fn load(options: FontOptions) -> LoadedFonts {
    // `FontSystem::new` sekaligus mendeteksi locale OS (penting untuk memilih
    // font Han yang benar). Mode deterministik melewatkannya dengan sengaja.
    let mut system = if options.system_fonts {
        FontSystem::new()
    } else {
        FontSystem::new_with_locale_and_db("en-US".to_string(), fontdb::Database::new())
    };

    let mut ui_family = None;
    if options.bundled_ui_font {
        let db = system.db_mut();
        let ids = db.load_font_source(fontdb::Source::Binary(Arc::new(BUNDLED_UI_FONT)));
        ui_family = ids
            .first()
            .and_then(|id| db.face(*id))
            .and_then(|face| face.families.first())
            .map(|(name, _)| name.clone());
        if let Some(name) = ui_family.clone() {
            // Sans-serif generik pun mengarah ke font UI kita, supaya teks tak
            // pernah jatuh ke default cosmic-text ("Open Sans") yang mungkin
            // tidak ada di mesin ini.
            db.set_sans_serif_family(name);
        }
    }

    LoadedFonts { system, ui_family }
}

/// Terjemahkan keluarga semantik framework ke keluarga cosmic-text.
pub(crate) fn family_for<'a>(family: &'a FontFamily, ui_family: Option<&'a str>) -> Family<'a> {
    match family {
        FontFamily::Ui => match ui_family {
            Some(name) => Family::Name(name),
            None => Family::SansSerif,
        },
        FontFamily::SansSerif => Family::SansSerif,
        FontFamily::Serif => Family::Serif,
        FontFamily::Monospace => Family::Monospace,
        FontFamily::Named(name) => Family::Name(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_bundel_ikut_ke_binary_dan_berbentuk_truetype() {
        assert!(
            BUNDLED_UI_FONT.len() > 100_000,
            "font bundel terlalu kecil: {}",
            BUNDLED_UI_FONT.len()
        );
        // Tag sfnt: 0x00010000 (TrueType) atau "OTTO".
        let tag = &BUNDLED_UI_FONT[..4];
        assert!(
            tag == [0x00, 0x01, 0x00, 0x00] || tag == b"OTTO",
            "tag: {tag:?}"
        );
    }

    #[test]
    fn mode_bundel_saja_hanya_memuat_satu_face() {
        let loaded = load(FontOptions::bundled_only());
        assert_eq!(loaded.system.db().len(), 1);
        let nama = loaded.ui_family.expect("nama keluarga terbaca dari font");
        assert!(nama.contains("Inter"), "nama keluarga tak terduga: {nama}");
    }

    #[test]
    fn keluarga_ui_menunjuk_font_bundel() {
        let loaded = load(FontOptions::bundled_only());
        let nama = loaded.ui_family.clone().unwrap();
        assert_eq!(
            family_for(&FontFamily::Ui, loaded.ui_family.as_deref()),
            Family::Name(&nama)
        );
    }

    #[test]
    fn tanpa_font_bundel_ui_jatuh_ke_sans_serif() {
        assert_eq!(family_for(&FontFamily::Ui, None), Family::SansSerif);
    }

    #[test]
    fn keluarga_generik_dipetakan_apa_adanya() {
        assert_eq!(family_for(&FontFamily::Monospace, None), Family::Monospace);
        assert_eq!(family_for(&FontFamily::Serif, None), Family::Serif);
        let brand = FontFamily::named("Acme Sans");
        assert_eq!(family_for(&brand, None), Family::Name("Acme Sans"));
    }
}
