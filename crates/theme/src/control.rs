//! Control heights, and the difference between what you see and what you hit.
//!
//! A button's height used to be whatever its text plus its padding came to. That
//! works until two controls have to line up: a `text_field` beside a `select`
//! beside a `button` agree only by coincidence, and the coincidence breaks the
//! moment one of them holds a longer word or a different type size.
//!
//! So height becomes a token, like every other number the design system owns
//! (§2.7). Three sizes for controls, one for a table or list row, one for a menu
//! row — and each derived from the spacing scale, so a preset that changes its
//! rhythm moves controls with it instead of leaving them behind:
//!
//! ```
//! use silka_theme::{Appearance, ControlToken, Theme};
//!
//! let t = Theme::cupertino(Appearance::Dark);
//!
//! // The medium control is the default, and the sizes are ordered.
//! assert!(t.control_of(ControlToken::Sm) < t.control_of(ControlToken::Md));
//! assert!(t.control_of(ControlToken::Md) < t.control_of(ControlToken::Lg));
//!
//! // A row is denser than a control: it is content, not something you press.
//! assert!(t.control_of(ControlToken::Row) < t.control_of(ControlToken::Md));
//! ```
//!
//! # Visual height is not the hit target
//!
//! This is the whole reason the module exists, and it is stated in the API rather
//! than left to be discovered.
//!
//! HIG asks for a hit target of at least 44pt ([`MIN_HIT_TARGET`]).
//! `KOMPONEN.md` makes that a line in the definition of done. But a 44pt-tall
//! button looks wrong in a dense table toolbar, and a design system that cannot
//! draw a small control is a design system applications will work around.
//!
//! Both are true at once, because they are different measurements:
//!
//! ```
//! use silka_theme::{Appearance, ControlToken, Theme, MIN_HIT_TARGET};
//!
//! let t = Theme::cupertino(Appearance::Dark);
//!
//! // A small control is deliberately shorter than the minimum target…
//! let visual = t.control_of(ControlToken::Sm);
//! assert!(visual < MIN_HIT_TARGET);
//!
//! // …and the area that responds to a finger is not.
//! assert_eq!(t.hit_target_of(ControlToken::Sm), MIN_HIT_TARGET);
//!
//! // Once the visual is tall enough, the target is simply the visual.
//! let lg = t.control_of(ControlToken::Lg);
//! assert_eq!(t.hit_target_of(ControlToken::Lg), lg.max(MIN_HIT_TARGET));
//! ```
//!
//! A widget draws [`Theme::control_of`](crate::Theme::control_of) and constrains itself to
//! [`Theme::hit_target_of`](crate::Theme::hit_target_of). The gap between them is padding that is felt and not
//! seen — which is exactly how a 28pt-tall macOS toolbar button still catches a
//! clumsy click.
//!
//! **`Row` and `MenuRow` are the exception, and it is a considered one.** A table
//! row is content, and a 200-row table cannot afford 44pt per row. Their hit
//! target is their own height; the *controls inside* a row keep their floor.

/// The smallest square a pointer or finger should have to hit (HIG).
///
/// Named here rather than in the widget crate because it is a rule about the
/// design system, not about one component — and because
/// [`Theme::hit_target_of`](crate::Theme::hit_target_of) has to be able to reach it.
pub const MIN_HIT_TARGET: f32 = 44.0;

/// Control and row heights, in logical points.
///
/// ```
/// use silka_theme::{ControlToken, ControlTokens};
///
/// let c = ControlTokens {
///     sm: 24.0,
///     md: 32.0,
///     lg: 40.0,
///     row: 28.0,
///     menu_row: 24.0,
/// };
/// assert_eq!(c.get(ControlToken::Md), 32.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlTokens {
    /// A compact control — a toolbar button, an inline filter.
    pub sm: f32,
    /// The default control height: button, `text_field`, `select`, `combo_box`.
    pub md: f32,
    /// A prominent control — a primary call to action, a search field.
    pub lg: f32,
    /// One row of a table or a list.
    pub row: f32,
    /// One row of a menu or a dropdown list.
    pub menu_row: f32,
}

impl ControlTokens {
    /// The height of one token, in logical points.
    pub fn get(self, token: ControlToken) -> f32 {
        match token {
            ControlToken::Sm => self.sm,
            ControlToken::Md => self.md,
            ControlToken::Lg => self.lg,
            ControlToken::Row => self.row,
            ControlToken::MenuRow => self.menu_row,
        }
    }

    /// The height of the area that must **respond**, as opposed to the area that
    /// is drawn.
    ///
    /// [`MIN_HIT_TARGET`] for anything a person presses, and the row's own height
    /// for a row — see the module docs for why a table row is exempt.
    pub fn hit_target(self, token: ControlToken) -> f32 {
        if token.is_row() {
            self.get(token)
        } else {
            self.get(token).max(MIN_HIT_TARGET)
        }
    }
}

/// A named control height.
///
/// ```
/// use silka_theme::ControlToken;
///
/// // Controls are pressed and therefore have a floor; rows are content.
/// assert!(!ControlToken::Md.is_row());
/// assert!(ControlToken::Row.is_row());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlToken {
    /// Compact — a toolbar button, an inline filter.
    Sm,
    /// The default control height.
    Md,
    /// Prominent — a primary action, a search field.
    Lg,
    /// One table or list row.
    Row,
    /// One menu or dropdown row.
    MenuRow,
}

impl ControlToken {
    /// Every token — for completeness tests and for a token dump.
    pub const ALL: [ControlToken; 5] = [
        ControlToken::Sm,
        ControlToken::Md,
        ControlToken::Lg,
        ControlToken::Row,
        ControlToken::MenuRow,
    ];

    /// The token's stable name, as it appears in a theme dump.
    pub const fn name(self) -> &'static str {
        match self {
            ControlToken::Sm => "sm",
            ControlToken::Md => "md",
            ControlToken::Lg => "lg",
            ControlToken::Row => "row",
            ControlToken::MenuRow => "menu_row",
        }
    }

    /// Whether this height describes a **row of content** rather than something
    /// a person presses.
    ///
    /// A row is exempt from the 44pt floor: a table with two hundred rows cannot
    /// spend 44pt on each, and a row is not a control. The controls *inside* a
    /// row are not exempt.
    pub const fn is_row(self) -> bool {
        matches!(self, ControlToken::Row | ControlToken::MenuRow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};

    #[test]
    fn urutan_tinggi_kontrol_masuk_akal() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let c = t.control;
            assert!(c.sm < c.md, "{preset:?}: sm harus lebih pendek dari md");
            assert!(c.md < c.lg, "{preset:?}: md harus lebih pendek dari lg");
            // A row is content: denser than the control that sits in it.
            assert!(c.row < c.md, "{preset:?}: baris harus lebih rapat dari md");
            assert!(c.menu_row <= c.row, "{preset:?}");
        }
    }

    #[test]
    fn setiap_tinggi_positif_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Dark);
            for token in ControlToken::ALL {
                assert!(
                    t.control_of(token) > 0.0,
                    "{preset:?}: {} tidak boleh nol",
                    token.name()
                );
            }
        }
    }

    /// The point of the module: the two measurements are allowed to disagree, and
    /// a control shorter than 44pt still answers a 44pt target.
    #[test]
    fn tinggi_visual_dan_hit_target_dua_hal_berbeda() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);

            // A small control is genuinely shorter than the floor…
            assert!(
                t.control_of(ControlToken::Sm) < MIN_HIT_TARGET,
                "{preset:?}: sm seharusnya lebih pendek dari 44pt, kalau tidak \
                 pemisahan ini tak ada gunanya"
            );
            // …and its target is not.
            for token in ControlToken::ALL {
                if token.is_row() {
                    continue;
                }
                assert!(
                    t.hit_target_of(token) >= MIN_HIT_TARGET,
                    "{preset:?}: {} melanggar batas 44pt HIG",
                    token.name()
                );
                assert!(
                    t.hit_target_of(token) >= t.control_of(token),
                    "{preset:?}: {} — target tidak boleh lebih kecil dari visual",
                    token.name()
                );
            }
        }
    }

    /// A row is the deliberate exception, and it has to stay one: if a row ever
    /// gained a 44pt floor, every dense table in the framework would silently
    /// become 60% taller.
    #[test]
    fn baris_dikecualikan_dari_batas_44pt() {
        let t = Theme::default();
        for token in [ControlToken::Row, ControlToken::MenuRow] {
            assert_eq!(t.hit_target_of(token), t.control_of(token));
            assert!(t.hit_target_of(token) < MIN_HIT_TARGET);
        }
    }

    #[test]
    fn nama_token_unik() {
        let mut nama: Vec<&str> = ControlToken::ALL.iter().map(|t| t.name()).collect();
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
    }

    #[test]
    fn tinggi_kontrol_mengikuti_ritme_spasi() {
        // Not decoration: a height off the scale is what makes a control look
        // "almost aligned" beside a padded box.
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let unit = t.spacing.unit;
            for token in ControlToken::ALL {
                let h = t.control_of(token);
                let steps = h / unit;
                assert!(
                    (steps - steps.round()).abs() < 1e-4,
                    "{preset:?}: {} = {h} bukan kelipatan {unit}",
                    token.name()
                );
            }
        }
    }

    #[test]
    fn geometri_tidak_ikut_dark_mode() {
        let terang = Theme::cupertino(Appearance::Light);
        let gelap = terang.with_appearance(Appearance::Dark);
        assert_eq!(terang.control, gelap.control);
    }
}
