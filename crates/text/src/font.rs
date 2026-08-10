//! Font sources: **bundled Inter** plus system fallback.
//!
//! Why Inter (REKOMENDASI §3.6): SF Pro cannot be shipped because of Apple's
//! license, whereas Inter is open (SIL OFL 1.1), feels very close to SF, and
//! version 4 carries an `opsz` (optical size) axis. What is bundled here is
//! `InterVariable.ttf` — a single variable font file covering the whole weight
//! range, so `weight(600)` is an axis setting rather than a second file.
//!
//! System fallback is still mandatory: Inter contains no CJK, Arabic, or emoji.
//! System fonts cover those (§3.3 "per-platform font fallback"), and cosmic-text
//! picks them per script.

use std::sync::Arc;

use cosmic_text::{Family, FontSystem};

use crate::style::FontFamily;

/// The UI font file bundled into the binary.
///
/// License: SIL Open Font License 1.1 — see `assets/fonts/LICENSE-Inter.txt`.
pub const BUNDLED_UI_FONT: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");

/// Where fonts come from when a [`crate::TextEngine`] is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontOptions {
    /// Load the bundled UI font (Inter). Almost always what you want.
    pub bundled_ui_font: bool,
    /// Load system fonts as fallback (CJK, Arabic, emoji).
    ///
    /// Turning this off makes results **deterministic** — which is what unit
    /// tests and CI use, since CI machines each have a different font list.
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
    /// Bundled font only — fast and deterministic, no fallback.
    pub fn bundled_only() -> Self {
        Self {
            bundled_ui_font: true,
            system_fonts: false,
        }
    }
}

/// The result of loading fonts.
pub(crate) struct LoadedFonts {
    pub(crate) system: FontSystem,
    /// The UI font's family name as reported by the file itself — read from the
    /// font rather than guessed as a constant.
    pub(crate) ui_family: Option<String>,
}

pub(crate) fn load(options: FontOptions) -> LoadedFonts {
    // `FontSystem::new` also detects the OS locale (which matters for picking
    // the right Han font). Deterministic mode skips that deliberately.
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
            // Generic sans-serif points at our UI font too, so text never falls
            // back to cosmic-text's default ("Open Sans"), which may not exist
            // on this machine.
            db.set_sans_serif_family(name);
        }
    }

    LoadedFonts { system, ui_family }
}

/// Translate the framework's semantic family into a cosmic-text family.
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
        // sfnt tag: 0x00010000 (TrueType) or "OTTO".
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
