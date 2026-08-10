//! **The categorical palette** — the one part of a chart that cannot be
//! delegated to the theme.
//!
//! Every other color in this crate is a semantic token ([`crate::style`]):
//! axis lines are `separator`, labels are `secondary_label`, the plot surface
//! is `surface`. Series colors are the exception, and deliberately so — they
//! do not encode a *role* ("this is the accent"), they encode **identity**
//! ("this is Revenue, and it stays Revenue when the other series is filtered
//! away"). A role palette cannot answer that: there is only one `accent`.
//!
//! ## Three rules this palette obeys
//!
//! 1. **Fixed order, never cycled.** Slot *n* always yields the same hue.
//!    A filter that removes a series must not repaint the survivors, so the
//!    color follows the series index, not its rank. Past
//!    [`CATEGORICAL_LEN`] slots the answer is *not* a generated hue — it is
//!    "fold the tail into Other, or split into small multiples"; the palette
//!    says so by wrapping back to slot 0 rather than inventing a color, and
//!    [`ChartPalette::is_exhausted`] lets a caller detect it.
//! 2. **Colorblind-safe by measurement, not by taste.** Adjacent slots are
//!    kept apart in OKLab under simulated protanopia/deuteranopia
//!    ([`cvd`]); the unit tests in this module recompute that distance rather
//!    than trusting a comment. See [`MIN_ADJACENT_CVD`] and
//!    [`MIN_ADJACENT_NORMAL`].
//! 3. **Light and dark are two selected sets, not one flipped set.** The dark
//!    column is the same eight hues re-stepped for a dark surface — chosen so
//!    that each still clears 3:1 against it — because automatically
//!    lightening a light-mode palette produces washed-out marks that stop
//!    being distinguishable exactly where they need to be.
//!
//! ## Why it does not depend on the preset
//!
//! [`Preset`](silka_theme::Preset) decides corner geometry, shadow recipe, and
//! the *role* palette. It deliberately does **not** decide the categorical
//! palette: CVD separation is a perceptual guarantee to the reader, not a
//! brand choice, and a chart that becomes unreadable because the app switched
//! from Cupertino to Tailwind would be a bug in both. What the preset still
//! governs is everything around the marks — surface, corners, labels — so a
//! chart continues to look like it belongs.
//!
//! An application with its own brand hues is not locked out: [`ChartPalette`]
//! is a plain value and [`ChartPalette::with_slots`] replaces its contents.
//! The obligation that comes with that is the same one this module met — run
//! the numbers.
//!
//! ```
//! use silka_chart::palette::ChartPalette;
//! use silka_theme::{Appearance, Theme};
//!
//! let palette = ChartPalette::for_theme(&Theme::cupertino(Appearance::Dark));
//! // Identity, not rank: slot 2 is slot 2 whatever else is on screen.
//! assert_eq!(palette.slot(2), palette.slot(2));
//! assert_ne!(palette.slot(0), palette.slot(1));
//! ```

use silka_paint::Color;
use silka_theme::{Appearance, Theme};

/// How many distinct categorical hues exist.
///
/// Eight is not a technical limit but a **perceptual** one: past eight slots no
/// ordering of any palette keeps every pair apart for a colorblind reader, and
/// a ninth generated hue would be a lie told in color. Past this count the
/// honest moves are "Other", small multiples, or a different encoding.
///
/// ```
/// use silka_chart::{ChartPalette, CATEGORICAL_LEN};
/// use silka_theme::{Appearance, Theme};
///
/// let palette = ChartPalette::for_theme(&Theme::cupertino(Appearance::Dark));
///
/// // Every slot within the count is a distinct hue…
/// let hues: Vec<_> = (0..CATEGORICAL_LEN).map(|i| palette.slot(i)).collect();
/// for i in 1..hues.len() {
///     assert_ne!(hues[i], hues[i - 1]);
/// }
///
/// // …and asking for more series than there are slots is a question the
/// // palette answers honestly rather than by inventing a ninth colour no
/// // colourblind reader could tell from an earlier one.
/// assert!(!palette.is_exhausted(CATEGORICAL_LEN));
/// assert!(palette.is_exhausted(CATEGORICAL_LEN + 1));
/// ```
pub const CATEGORICAL_LEN: usize = 8;

/// The minimum OKLab distance (×100) required between **adjacent** slots under
/// simulated protanopia/deuteranopia.
///
/// Adjacent is the pair that matters for lines, bars, and stacks: those are the
/// marks that end up side by side. Enforced by the tests in this module.
pub const MIN_ADJACENT_CVD: f32 = 8.0;

/// The minimum OKLab distance (×100) required between adjacent slots for
/// **normal** vision.
///
/// A separate floor, because a palette can be safe for a colorblind reader and
/// still be muddy for everyone else.
pub const MIN_ADJACENT_NORMAL: f32 = 15.0;

/// The eight light-mode hues, in slot order.
const LIGHT: [u32; CATEGORICAL_LEN] = [
    0x2A78D6, // 1 blue
    0xEB6834, // 2 orange
    0x1BAF7A, // 3 aqua
    0xEDA100, // 4 yellow
    0xE87BA4, // 5 magenta
    0x008300, // 6 green
    0x4A3AA7, // 7 violet
    0xE34948, // 8 red
];

/// The eight dark-mode hues, in the same slot order — re-stepped for a dark
/// surface, not lightened algorithmically.
const DARK: [u32; CATEGORICAL_LEN] = [
    0x3987E5, // 1 blue
    0xD95926, // 2 orange
    0x199E70, // 3 aqua
    0xC98500, // 4 yellow
    0xD55181, // 5 magenta
    // Green is the one slot whose light-mode value would otherwise be reused
    // verbatim in dark mode. It is re-stepped here because it has to clear 3:1
    // against **our** darkest-but-lightest surface (`surface` under Cupertino
    // Dark, #2C2C2E) — a lighter ground than the reference palette assumed, and
    // the one that actually decides the number. The test below is what keeps
    // this claim true against every preset's dark surfaces.
    0x008E00, // 6 green
    0x9085E9, // 7 violet
    0xE66767, // 8 red
];

/// The categorical palette in effect for one chart.
///
/// A plain value like [`Theme`] itself: it is rebuilt when the appearance
/// changes rather than invalidated, so there is no hidden state to go stale
/// when the OS switches to dark mode.
///
/// ```
/// use silka_chart::palette::{cvd, ChartPalette, CATEGORICAL_LEN, MIN_ADJACENT_CVD};
/// use silka_theme::Appearance;
///
/// let palette = ChartPalette::for_appearance(Appearance::Dark);
/// assert_eq!(palette.slots().len(), CATEGORICAL_LEN);
///
/// // Eight slots is the honest limit: a ninth series would repeat a color,
/// // so the caller is told rather than quietly misled.
/// assert!(!palette.is_exhausted(8));
/// assert!(palette.is_exhausted(9));
///
/// // Neighbouring slots stay apart under red- and green-blindness — the
/// // promise this palette exists to keep, and it is checked by arithmetic.
/// let (a, b) = (palette.slot(0), palette.slot(1));
/// assert!(cvd::worst_required(a, b) >= MIN_ADJACENT_CVD);
///
/// // An area fill is the line color tinted down, not a second token.
/// assert!(palette.fill(a).a < a.a);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartPalette {
    slots: [Color; CATEGORICAL_LEN],
    /// How strongly an area fill is tinted down from its line color.
    fill_alpha: f32,
    /// How strongly a de-emphasised (not hovered) series is faded.
    muted_alpha: f32,
}

impl ChartPalette {
    /// The palette for a theme — chosen by its [`Appearance`], not its preset
    /// (see the module docs).
    pub fn for_theme(theme: &Theme) -> Self {
        Self::for_appearance(theme.appearance)
    }

    /// The palette for one appearance.
    pub fn for_appearance(appearance: Appearance) -> Self {
        let hex = match appearance {
            Appearance::Light => LIGHT,
            Appearance::Dark => DARK,
        };
        let mut slots = [Color::TRANSPARENT; CATEGORICAL_LEN];
        for (slot, rgb) in slots.iter_mut().zip(hex) {
            *slot = Color::hex(rgb);
        }
        Self {
            slots,
            // A fill sits *behind* its own line and behind the gridlines; at
            // full strength it would out-shout the line that carries the shape.
            fill_alpha: 0.18,
            muted_alpha: 0.35,
        }
    }

    /// The color of one series, by **index**.
    ///
    /// Wraps past [`CATEGORICAL_LEN`] instead of generating a new hue — see the
    /// module docs on why a ninth color would be a lie, and
    /// [`ChartPalette::is_exhausted`] for detecting it.
    pub fn slot(&self, index: usize) -> Color {
        self.slots[index % CATEGORICAL_LEN]
    }

    /// True when `count` series no longer fit into distinct hues.
    ///
    /// Not an error and not a panic: it is the signal that the *data* needs a
    /// different shape (fold to "Other", facet into small multiples), which is
    /// a decision only the application can take.
    pub fn is_exhausted(&self, count: usize) -> bool {
        count > CATEGORICAL_LEN
    }

    /// Every slot, in order.
    pub fn slots(&self) -> &[Color; CATEGORICAL_LEN] {
        &self.slots
    }

    /// The area-fill color derived from a series color.
    pub fn fill(&self, color: Color) -> Color {
        color.with_alpha(color.a * self.fill_alpha)
    }

    /// A de-emphasised version of a series color (everything except the series
    /// under the pointer).
    pub fn muted(&self, color: Color) -> Color {
        color.with_alpha(color.a * self.muted_alpha)
    }

    /// A palette with its hues replaced — the door for a brand palette.
    ///
    /// Whoever walks through it inherits the obligation the default met:
    /// validate the new hues (this module's tests are written against
    /// [`ChartPalette`], so pointing them at a custom palette is a two-line
    /// change).
    pub fn with_slots(mut self, slots: [Color; CATEGORICAL_LEN]) -> Self {
        self.slots = slots;
        self
    }

    /// A palette with a different area-fill strength (0…1).
    pub fn with_fill_alpha(mut self, alpha: f32) -> Self {
        self.fill_alpha = alpha.clamp(0.0, 1.0);
        self
    }
}

impl Default for ChartPalette {
    fn default() -> Self {
        Self::for_appearance(Appearance::Light)
    }
}

// ---------------------------------------------------------------------------
// Color-vision-deficiency math
// ---------------------------------------------------------------------------

/// Perceptual distance and colorblind simulation — the arithmetic that turns
/// "looks distinguishable" into a number a test can fail on.
///
/// The simulation is Machado, Oliveira & Fernandes (2009) at severity 1.0,
/// applied in **linear** RGB; the distance is Euclidean in OKLab ×100. Both
/// choices matter and neither is arbitrary: a simulation applied to sRGB values
/// exaggerates separation in the dark end, and a distance measured in sRGB
/// space rates two dark hues as far apart when a reader cannot tell them apart
/// at all.
pub mod cvd {
    use silka_paint::{srgb_to_linear, Color};

    /// A form of color-vision deficiency.
    ///
    /// ```
    /// use silka_chart::palette::cvd::{self, Deficiency};
    /// use silka_paint::Color;
    ///
    /// // Red and green are far apart to a typical reader…
    /// let (red, green) = (Color::hex(0xD62728), Color::hex(0x2CA02C));
    /// assert!(cvd::delta_e(red, green) > 20.0);
    /// // …and much closer to a green-blind one, which is exactly the mistake
    /// // a hand-picked "red vs green" palette makes.
    /// assert!(cvd::delta_e_cvd(red, green, Deficiency::Deuteranopia) < cvd::delta_e(red, green));
    ///
    /// // The palette is gated on the two common forms, not all three:
    /// // requiring tritanopia at eight slots leaves no palette standing.
    /// assert_eq!(Deficiency::REQUIRED.len(), 2);
    /// assert_eq!(
    ///     cvd::worst_required(red, green),
    ///     cvd::delta_e_cvd(red, green, Deficiency::Protanopia)
    ///         .min(cvd::delta_e_cvd(red, green, Deficiency::Deuteranopia))
    /// );
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Deficiency {
        /// Red-blind (~1% of men).
        Protanopia,
        /// Green-blind (~1% of men) — the most common form.
        Deuteranopia,
        /// Blue-blind (rare, and not sex-linked).
        Tritanopia,
    }

    impl Deficiency {
        /// The two forms a categorical palette is **required** to survive.
        ///
        /// Tritanopia is reported but not gated: it is far rarer, and gating on
        /// all three at eight slots leaves no palette standing.
        pub const REQUIRED: [Deficiency; 2] = [Deficiency::Protanopia, Deficiency::Deuteranopia];

        /// The Machado et al. transform matrix (row-major, linear RGB).
        fn matrix(self) -> [[f32; 3]; 3] {
            match self {
                Deficiency::Protanopia => [
                    [0.152_286, 1.052_583, -0.204_868],
                    [0.114_503, 0.786_281, 0.099_216],
                    [-0.003_882, -0.048_116, 1.051_998],
                ],
                Deficiency::Deuteranopia => [
                    [0.367_322, 0.860_646, -0.227_968],
                    [0.280_085, 0.672_501, 0.047_413],
                    [-0.011_820, 0.042_940, 0.968_881],
                ],
                Deficiency::Tritanopia => [
                    [1.255_528, -0.076_749, -0.178_779],
                    [-0.078_411, 0.930_809, 0.147_602],
                    [0.004_733, 0.691_367, 0.303_900],
                ],
            }
        }
    }

    /// A color in linear RGB, as seen through `deficiency`.
    pub fn simulate(color: Color, deficiency: Deficiency) -> [f32; 3] {
        let [r, g, b, _] = color.to_linear();
        let m = deficiency.matrix();
        let mut out = [0.0; 3];
        for (row, coeff) in out.iter_mut().zip(m) {
            *row = (coeff[0] * r + coeff[1] * g + coeff[2] * b).clamp(0.0, 1.0);
        }
        out
    }

    /// OKLab coordinates of a **linear** RGB triple.
    pub fn oklab(linear: [f32; 3]) -> [f32; 3] {
        let [r, g, b] = linear;
        let l = (0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b).cbrt();
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }

    /// The OKLab distance (×100) between two colors as seen by normal vision.
    pub fn delta_e(a: Color, b: Color) -> f32 {
        let (a, b) = (drop_alpha(a), drop_alpha(b));
        distance(oklab(a), oklab(b))
    }

    /// The OKLab distance (×100) between two colors as seen through
    /// `deficiency`.
    pub fn delta_e_cvd(a: Color, b: Color, deficiency: Deficiency) -> f32 {
        distance(
            oklab(simulate(a, deficiency)),
            oklab(simulate(b, deficiency)),
        )
    }

    /// The worst case over [`Deficiency::REQUIRED`] — the number a palette is
    /// gated on.
    pub fn worst_required(a: Color, b: Color) -> f32 {
        Deficiency::REQUIRED
            .iter()
            .map(|d| delta_e_cvd(a, b, *d))
            .fold(f32::INFINITY, f32::min)
    }

    /// The WCAG contrast ratio between two opaque colors.
    ///
    /// A mark below 3:1 against the chart surface is not forbidden, but it
    /// obliges the chart to carry a second encoding — which is exactly why
    /// legends and direct labels are not optional in this crate.
    pub fn contrast(a: Color, b: Color) -> f32 {
        let lum = |c: Color| {
            let [r, g, b, _] = c.to_linear();
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (x, y) = (lum(a), lum(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn drop_alpha(c: Color) -> [f32; 3] {
        let [r, g, b, _] = c.components();
        [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)]
    }

    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        100.0 * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::cvd::{contrast, delta_e, worst_required};
    use super::*;

    fn palet(appearance: Appearance) -> ChartPalette {
        ChartPalette::for_appearance(appearance)
    }

    #[test]
    fn slot_stabil_dan_tidak_bergantung_pada_peringkat() {
        // The property that makes a filter safe: removing a series must not
        // repaint the ones that remain. A palette that answered "the colors of
        // the first n series" instead of "the color of slot i" could not
        // promise this.
        let p = palet(Appearance::Light);
        for i in 0..CATEGORICAL_LEN {
            assert_eq!(p.slot(i), p.slots()[i]);
        }
        assert_eq!(
            p.slot(0),
            p.slot(CATEGORICAL_LEN),
            "slot melingkar, bukan warna baru"
        );
        assert!(!p.is_exhausted(CATEGORICAL_LEN));
        assert!(p.is_exhausted(CATEGORICAL_LEN + 1));
    }

    #[test]
    fn setiap_slot_berbeda_di_kedua_appearance() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let p = palet(appearance);
            for i in 0..CATEGORICAL_LEN {
                for j in (i + 1)..CATEGORICAL_LEN {
                    assert_ne!(p.slot(i), p.slot(j), "{appearance:?}: slot {i} == slot {j}");
                }
            }
        }
    }

    #[test]
    fn pasangan_bertetangga_aman_untuk_buta_warna() {
        // The heart of the matter, and the reason this is arithmetic instead of
        // a comment: adjacent slots are the ones that end up side by side in a
        // stack, a grouped bar, or a legend. If protan/deutan vision collapses
        // that pair, the chart is unreadable for ~1 in 12 men — and nobody
        // filing the bug will be able to describe it.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let p = palet(appearance);
            for i in 0..CATEGORICAL_LEN - 1 {
                let d = worst_required(p.slot(i), p.slot(i + 1));
                assert!(
                    d >= MIN_ADJACENT_CVD,
                    "{appearance:?}: slot {i}↔{} hanya ΔE {d:.1} (minimal {MIN_ADJACENT_CVD})",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn pasangan_bertetangga_juga_jelas_untuk_penglihatan_normal() {
        // A palette can be CVD-safe and still muddy for everyone else; that is a
        // separate floor, so it gets a separate test.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let p = palet(appearance);
            for i in 0..CATEGORICAL_LEN - 1 {
                let d = delta_e(p.slot(i), p.slot(i + 1));
                assert!(
                    d >= MIN_ADJACENT_NORMAL,
                    "{appearance:?}: slot {i}↔{} hanya ΔE {d:.1}",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn tiga_slot_pertama_aman_untuk_semua_pasangan() {
        // Scatter plots and small multiples put *every* pair on screen at once,
        // not just neighbours. Under that harsher rule the palette carries a
        // documented cap of three — this test is what keeps that claim true.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let p = palet(appearance);
            for i in 0..3 {
                for j in (i + 1)..3 {
                    let d = worst_required(p.slot(i), p.slot(j));
                    assert!(d >= MIN_ADJACENT_CVD, "{appearance:?}: {i}↔{j} ΔE {d:.1}");
                }
            }
        }
    }

    #[test]
    fn slot_gelap_kontras_terhadap_setiap_permukaan_gelap() {
        // The dark column exists precisely because flipping the light one does
        // not work. Which ground it has to clear is not a matter of opinion
        // either: it is whichever of the two presets' dark surfaces is
        // *lightest*, since that is where a mark has the least room. Checking
        // one preset would have let the other one ship an invisible series.
        for preset in silka_theme::Preset::ALL {
            let t = Theme::new(preset, Appearance::Dark);
            let p = ChartPalette::for_theme(&t);
            for i in 0..CATEGORICAL_LEN {
                for (nama, latar) in [
                    ("surface", t.color.surface),
                    ("background", t.color.background),
                ] {
                    let rasio = contrast(p.slot(i), latar);
                    assert!(
                        rasio >= 3.0,
                        "{preset:?}: slot {i} hanya {rasio:.2}:1 di atas {nama}"
                    );
                }
            }
        }
    }

    #[test]
    fn slot_terang_kontras_terhadap_permukaan_terang_atau_diberi_kelonggaran() {
        // Three light-mode hues (aqua, yellow, magenta) sit below 3:1 on a white
        // surface, and that is a *documented* relaxation rather than an
        // oversight: it is legal only because this crate never lets identity
        // rest on color alone — every chart with more than one series carries a
        // legend, and the tooltip names each one. The test pins how many are
        // allowed to take that relief, so a fourth cannot appear unnoticed.
        for preset in silka_theme::Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            let p = ChartPalette::for_theme(&t);
            let lemah = (0..CATEGORICAL_LEN)
                .filter(|i| contrast(p.slot(*i), t.color.surface) < 3.0)
                .count();
            assert!(
                lemah <= 3,
                "{preset:?}: {lemah} slot butuh kelonggaran kontras"
            );
        }
    }

    #[test]
    fn light_dan_dark_adalah_dua_himpunan_terpilih() {
        let terang = palet(Appearance::Light);
        let gelap = palet(Appearance::Dark);
        let berbeda = (0..CATEGORICAL_LEN)
            .filter(|i| terang.slot(*i) != gelap.slot(*i))
            .count();
        assert!(
            berbeda >= CATEGORICAL_LEN - 1,
            "hampir setiap slot harus dipilih ulang untuk permukaan gelap"
        );
    }

    #[test]
    fn simulasi_buta_warna_meruntuhkan_pasangan_yang_memang_runtuh() {
        // A guard on the guard: if the simulation were the identity function
        // (a plausible way to silently break the matrix), the test above would
        // pass no matter how bad the palette was. Red vs green must collapse.
        let merah = Color::hex(0xD00000);
        let hijau = Color::hex(0x00A000);
        let normal = delta_e(merah, hijau);
        let deutan = worst_required(merah, hijau);
        assert!(normal > 30.0, "normal ΔE {normal:.1}");
        assert!(
            deutan < normal * 0.5,
            "deutan ΔE {deutan:.1} vs normal {normal:.1}"
        );
    }

    #[test]
    fn oklab_sepakat_dengan_titik_acuan() {
        // White is L=1 with no chroma; a bug in the matrix shows up here first.
        let [l, a, b] = super::cvd::oklab([1.0, 1.0, 1.0]);
        assert!((l - 1.0).abs() < 0.001, "L={l}");
        assert!(a.abs() < 0.001 && b.abs() < 0.001, "a={a} b={b}");
        assert!(delta_e(Color::WHITE, Color::WHITE) < 1e-4);
    }

    #[test]
    fn isi_area_lebih_pudar_daripada_garisnya() {
        let p = palet(Appearance::Dark);
        let garis = p.slot(0);
        assert!(p.fill(garis).a < garis.a);
        assert!(p.muted(garis).a < garis.a);
        assert_eq!(
            p.with_fill_alpha(2.0).fill(garis).a,
            garis.a,
            "alpha dijepit ke 1"
        );
    }

    #[test]
    fn palet_brand_bisa_menggantikan_isinya() {
        let p = palet(Appearance::Light).with_slots([Color::hex(0x7C3AED); CATEGORICAL_LEN]);
        assert_eq!(p.slot(3), Color::hex(0x7C3AED));
    }

    #[test]
    fn palet_sama_untuk_kedua_preset() {
        // Documented on purpose: CVD safety is a promise to the reader, not a
        // brand decision, so switching preset must not touch the marks.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let cup = ChartPalette::for_theme(&Theme::cupertino(appearance));
            let tw = ChartPalette::for_theme(&Theme::tailwind(appearance));
            assert_eq!(cup, tw, "{appearance:?}");
        }
    }
}
