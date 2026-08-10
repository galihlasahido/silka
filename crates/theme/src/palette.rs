//! The **raw** palette — color numbers as-is, with no meaning yet.
//!
//! This layer is deliberately kept apart from the semantic tokens
//! ([`crate::ColorTokens`]): it is the one and only place color literals are
//! allowed to live. Presets read this palette and assign roles (`surface`,
//! `accent`, …); widgets never touch this module at all (REKOMENDASI §2.6,
//! §2.7).
//!
//! Two sources of numbers:
//!
//! - [`tailwind`] — 11-step 50–950 ramps, copied verbatim from the Tailwind
//!   palette. This is the "Tailwind look" people recognize: not its CSS, but
//!   its numbers (§2.6).
//! - [`hig`] — Apple system colors (systemBlue, label, separator, fill) along
//!   with their light/dark pairs.
//!
//! The values are stored as `u32` hex literals rather than [`Color`] so they
//! can be `const` without float arithmetic in a `const fn` (a limit of the
//! workspace `rust-version`).
//!
//! ```
//! use silka_theme::palette::{hig, tailwind, Step};
//!
//! // A ramp is indexed by its Tailwind step number, not by an integer nobody
//! // can read at a glance.
//! let mid = tailwind::BLUE.get(Step::S500);
//! let deep = tailwind::BLUE.get(Step::S900);
//! assert!(deep.r < mid.r); // 900 really is the darker end
//!
//! // Apple's colors come as explicit light/dark pairs — the dark variant is
//! // brighter, because it sits on a dark surface.
//! assert_ne!(hig::SYSTEM_BLUE_LIGHT, hig::SYSTEM_BLUE_DARK);
//!
//! // Nothing here has any meaning yet: assigning `SYSTEM_BLUE_DARK` the role
//! // of "accent" is the preset's job, and widgets see only the role.
//! for step in Step::ALL {
//!     let _ = tailwind::SLATE.get(step);
//! }
//! ```

use silka_paint::Color;

/// One step on a 50–950 ramp.
///
/// The order matches how the palette is written: [`Step::S50`] is the lightest,
/// [`Step::S950`] the darkest.
///
/// ```
/// use silka_theme::palette::{tailwind, Step};
///
/// assert_eq!(Step::S500.value(), 500);
/// assert_eq!(Step::S50.index(), 0);
/// assert!(Step::S50 < Step::S950);
///
/// // Ramps are the only place color literals live; semantic tokens borrow
/// // from them, and widgets never see either.
/// let slate = tailwind::SLATE;
/// assert_eq!(slate.shades().len(), 11);
/// // Step 50 is nearly white, step 950 nearly black.
/// assert!(slate.get(Step::S50).r > slate.get(Step::S950).r);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    /// Step 50 — nearly white.
    S50,
    /// Step 100.
    S100,
    /// Step 200.
    S200,
    /// Step 300.
    S300,
    /// Step 400.
    S400,
    /// Step 500 — the midpoint of the ramp.
    S500,
    /// Step 600.
    S600,
    /// Step 700.
    S700,
    /// Step 800.
    S800,
    /// Step 900.
    S900,
    /// Step 950 — nearly black.
    S950,
}

impl Step {
    /// Every step, lightest to darkest.
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

    /// The step number as people write it (`slate-500` → `500`).
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

    /// The step's position inside a [`Ramp`] array.
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

/// An 11-step color ramp (50–950).
///
/// ```
/// use silka_theme::palette::{tailwind, Step};
///
/// // `bg-slate-800` on the web = this color, with no CSS involved.
/// assert_eq!(tailwind::SLATE.hex(Step::S800), 0x1E293B);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ramp([u32; 11]);

impl Ramp {
    /// A ramp from 11 hex literals, ordered 50 → 950.
    pub const fn new(hex: [u32; 11]) -> Self {
        Self(hex)
    }

    /// The hex literal for one step.
    pub const fn hex(self, step: Step) -> u32 {
        self.0[step.index()]
    }

    /// The color of one step.
    pub fn get(self, step: Step) -> Color {
        Color::hex(self.hex(step))
    }

    /// The whole ramp as colors, ordered 50 → 950.
    pub fn shades(self) -> [Color; 11] {
        let mut out = [Color::TRANSPARENT; 11];
        for (i, step) in Step::ALL.iter().enumerate() {
            out[i] = self.get(*step);
        }
        out
    }
}

/// The Tailwind ramps used by the `Tailwind/shadcn` preset (§2.7).
///
/// `slate` is shadcn/ui's neutral; `blue` is its accent. Three further ramps
/// serve the status tokens (destructive/success/warning) so this preset never
/// has to borrow a color from HIG.
pub mod tailwind {
    use super::Ramp;

    /// Blue-tinted neutral — the basis of every surface and every piece of
    /// text in this preset.
    pub const SLATE: Ramp = Ramp::new([
        0xF8FAFC, 0xF1F5F9, 0xE2E8F0, 0xCBD5E1, 0x94A3B8, 0x64748B, 0x475569, 0x334155, 0x1E293B,
        0x0F172A, 0x020617,
    ]);

    /// shadcn/ui's primary accent.
    pub const BLUE: Ramp = Ramp::new([
        0xEFF6FF, 0xDBEAFE, 0xBFDBFE, 0x93C5FD, 0x60A5FA, 0x3B82F6, 0x2563EB, 0x1D4ED8, 0x1E40AF,
        0x1E3A8A, 0x172554,
    ]);

    /// Destructive actions.
    pub const RED: Ramp = Ramp::new([
        0xFEF2F2, 0xFEE2E2, 0xFECACA, 0xFCA5A5, 0xF87171, 0xEF4444, 0xDC2626, 0xB91C1C, 0x991B1B,
        0x7F1D1D, 0x450A0A,
    ]);

    /// Success state.
    pub const EMERALD: Ramp = Ramp::new([
        0xECFDF5, 0xD1FAE5, 0xA7F3D0, 0x6EE7B7, 0x34D399, 0x10B981, 0x059669, 0x047857, 0x065F46,
        0x064E3B, 0x022C22,
    ]);

    /// Warning state.
    pub const AMBER: Ramp = Ramp::new([
        0xFFFBEB, 0xFEF3C7, 0xFDE68A, 0xFCD34D, 0xFBBF24, 0xF59E0B, 0xD97706, 0xB45309, 0x92400E,
        0x78350F, 0x451A03,
    ]);
}

/// Apple system colors (HIG) for the `Cupertino` preset.
///
/// Apple publishes a light/dark **pair** for every color — not one color that
/// gets darkened automatically. That is why each constant here has a `_LIGHT`
/// and a `_DARK` variant, and the preset picks based on
/// [`crate::Appearance`].
///
/// HIG's label/separator/fill colors are **semi-transparent** (they blend into
/// the material behind them). Their alpha is kept separately as `*_ALPHA`
/// constants so it can be applied via
/// [`silka_paint::Color::with_alpha`].
pub mod hig {
    /// systemBlue — the default macOS/iOS accent color (light).
    pub const SYSTEM_BLUE_LIGHT: u32 = 0x007AFF;
    /// systemBlue (dark).
    pub const SYSTEM_BLUE_DARK: u32 = 0x0A84FF;
    /// systemBlue one notch deeper — used for hover in light mode.
    pub const SYSTEM_BLUE_PRESSED_LIGHT: u32 = 0x0069DB;
    /// systemBlue one notch lighter — hover in dark mode.
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

    /// systemGroupedBackground (light) — the Settings-style window background.
    pub const GROUPED_BACKGROUND_LIGHT: u32 = 0xF2F2F7;
    /// Window background (dark).
    pub const GROUPED_BACKGROUND_DARK: u32 = 0x1C1C1E;
    /// secondarySystemGroupedBackground (light) — the card surface.
    pub const SURFACE_LIGHT: u32 = 0xFFFFFF;
    /// Card surface (dark).
    pub const SURFACE_DARK: u32 = 0x2C2C2E;
    /// tertiarySystemGroupedBackground (dark) — the raised surface.
    pub const SURFACE_ELEVATED_DARK: u32 = 0x3A3A3C;
    /// The "recessed" surface (dark), e.g. a scroll-area floor.
    pub const SURFACE_SUNKEN_DARK: u32 = 0x141416;
    /// The "recessed" surface (light).
    pub const SURFACE_SUNKEN_LIGHT: u32 = 0xE9E9EE;

    /// Base label color in light mode — pure black; alpha distinguishes the
    /// levels.
    pub const LABEL_LIGHT: u32 = 0x000000;
    /// Base label color in dark mode.
    pub const LABEL_DARK: u32 = 0xFFFFFF;
    /// Base secondary/tertiary label color in light mode (`#3C3C43`).
    pub const LABEL_TINT_LIGHT: u32 = 0x3C3C43;
    /// Base secondary/tertiary label color in dark mode (`#EBEBF5`).
    pub const LABEL_TINT_DARK: u32 = 0xEBEBF5;

    /// secondaryLabel alpha.
    pub const SECONDARY_LABEL_ALPHA: f32 = 0.60;
    /// tertiaryLabel alpha.
    pub const TERTIARY_LABEL_ALPHA: f32 = 0.30;
    /// quaternaryLabel alpha — disabled text.
    pub const QUATERNARY_LABEL_ALPHA: f32 = 0.18;

    /// Base separator color (light).
    pub const SEPARATOR_LIGHT: u32 = 0x3C3C43;
    /// Separator alpha (light).
    pub const SEPARATOR_ALPHA_LIGHT: f32 = 0.29;
    /// Base separator color (dark).
    pub const SEPARATOR_DARK: u32 = 0x545458;
    /// Separator alpha (dark).
    pub const SEPARATOR_ALPHA_DARK: f32 = 0.65;

    /// systemFill — the transient control background (light).
    pub const FILL_LIGHT: u32 = 0x787880;
    /// systemFill (dark).
    pub const FILL_DARK: u32 = 0x7C7C80;
    /// quaternarySystemFill alpha — used for surface hover.
    pub const FILL_HOVER_ALPHA: f32 = 0.12;
    /// tertiarySystemFill alpha — used for the pressed state.
    pub const FILL_PRESSED_ALPHA: f32 = 0.20;

    /// Modal scrim (the dim behind a sheet/dialog), light.
    pub const SCRIM_ALPHA_LIGHT: f32 = 0.20;
    /// Modal scrim, dark.
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
        // These numbers are what makes the "Tailwind look" — if they drift,
        // the second preset loses its reason to exist (§2.6).
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
