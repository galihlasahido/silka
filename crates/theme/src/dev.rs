//! **Design tokens in a text file, reloaded without a restart** (REKOMENDASI §9.1).
//!
//! §9.1 calls developer experience "the biggest danger": choosing Dart-style
//! Rust costs us Flutter's sub-second hot reload, and polish comes from
//! thousands of small iterations. Recompiling to move a padding by two points is
//! how a design system dies of attrition.
//!
//! This module closes the cheapest and largest part of that gap, and only that
//! part: **every value the utility vocabulary can produce comes from a
//! [`Theme`]** (§2.6), so making the theme loadable from a file makes the whole
//! visual surface editable while the application runs. No recompilation, no
//! restart, no code reload — the shell re-reads the file, writes the
//! `Signal<Theme>`, and the components that read the theme rebuild themselves.
//!
//! What it deliberately does **not** claim: this is not hot reload. Layout
//! structure, event handlers, and new widgets still need a compile. The skeleton
//! for that lives in `silka_core::hot`, and it is honest about being a skeleton.
//!
//! # The format
//!
//! A deliberately tiny `key = value` list — no dependency, no nesting, no
//! quoting rules to learn, and every error carries a line number:
//!
//! ```text
//! # silka theme tokens
//! preset = cupertino
//! appearance = dark
//!
//! color.accent = #0A84FF
//! color.surface = #1C1C1E
//! color.scrim = #00000066     # RGBA is allowed too
//!
//! radius.style = squircle     # or: arc, squircle(0.6)
//! radius.md = 8
//! space.unit = 4
//! font.body.size = 13
//! ```
//!
//! `preset` and `appearance` choose the **base** theme; everything after them is
//! an override on top of it. That ordering is what lets a file be three lines
//! long: start from Tailwind dark, change the accent, done.
//!
//! ```
//! use silka_theme::dev::ThemeOverrides;
//! use silka_theme::{Appearance, ColorToken, Preset, Theme};
//! use silka_paint::Color;
//!
//! let text = "\
//! preset = tailwind
//! appearance = dark
//! color.accent = #FF5F1F
//! radius.md = 10
//! ";
//!
//! let overrides = ThemeOverrides::parse(text).expect("valid");
//! let theme = overrides.apply(Theme::default());
//!
//! assert_eq!(theme.preset, Preset::Tailwind);
//! assert_eq!(theme.appearance, Appearance::Dark);
//! assert_eq!(theme.color.get(ColorToken::Accent), Color::hex(0xFF5F1F));
//! assert_eq!(theme.radius.md, 10.0);
//! ```
//!
//! # Getting a file to start from
//!
//! [`ThemeOverrides::dump`] writes the **whole** active theme out in this
//! format, which is how a designer gets a complete, correct starting point
//! instead of a blank file and a guess:
//!
//! ```
//! use silka_theme::dev::ThemeOverrides;
//! use silka_theme::{Appearance, Theme};
//!
//! let theme = Theme::cupertino(Appearance::Dark);
//! let text = ThemeOverrides::dump(&theme);
//!
//! // And it round-trips: what comes back out is the theme that went in.
//! let same = ThemeOverrides::parse(&text).unwrap().apply(Theme::default());
//! assert_eq!(same.radius, theme.radius);
//! assert_eq!(ThemeOverrides::dump(&same), text);
//! ```

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use silka_paint::{Color, CornerStyle};

use crate::{
    Appearance, ColorToken, ControlToken, FontToken, Preset, RadiusToken, Theme, TypeStyle,
    TypographyTokens,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A parse failure, with the line it happened on.
///
/// The line number is the whole point: a token file is edited by hand, often by
/// somebody who does not read Rust, and "something is wrong" is not a usable
/// message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenParseError {
    /// 1-based line number.
    pub line: usize,
    /// What was wrong, in one sentence.
    pub message: String,
}

impl fmt::Display for TokenParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TokenParseError {}

/// Why a [`ThemeFile`] could not be turned into overrides.
#[derive(Debug)]
pub enum ThemeFileError {
    /// The file could not be read.
    Io(std::io::Error),
    /// The file was read but does not parse.
    Parse(TokenParseError),
}

impl fmt::Display for ThemeFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeFileError::Io(e) => write!(f, "the token file could not be read: {e}"),
            ThemeFileError::Parse(e) => write!(f, "the token file is invalid — {e}"),
        }
    }
}

impl std::error::Error for ThemeFileError {}

impl From<std::io::Error> for ThemeFileError {
    fn from(e: std::io::Error) -> Self {
        ThemeFileError::Io(e)
    }
}

impl From<TokenParseError> for ThemeFileError {
    fn from(e: TokenParseError) -> Self {
        ThemeFileError::Parse(e)
    }
}

// ---------------------------------------------------------------------------
// ThemeOverrides
// ---------------------------------------------------------------------------

/// A parsed token file: a base theme to start from, plus the values to change.
///
/// Ordered rather than a map, so applying twice is applying the file twice —
/// there is no hidden "last one wins across reloads" state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThemeOverrides {
    preset: Option<Preset>,
    appearance: Option<Appearance>,
    colors: Vec<(ColorToken, Color)>,
    radii: Vec<(RadiusToken, f32)>,
    corner_style: Option<CornerStyle>,
    space_unit: Option<f32>,
    controls: Vec<(ControlToken, f32)>,
    font_sizes: Vec<(FontToken, f32)>,
}

impl ThemeOverrides {
    /// Parse the format described in the module docs.
    pub fn parse(text: &str) -> Result<Self, TokenParseError> {
        let mut out = Self::default();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, rest)) = trimmed.split_once('=') else {
                return Err(TokenParseError {
                    line,
                    message: format!("expected `key = value`, found {trimmed:?}"),
                });
            };
            // A comment may follow a value ("# too dark on the projector") — but
            // `#` also *starts* a color literal, so in that one case the comment
            // begins at the second `#`.
            let value_raw = rest.trim();
            let value = match value_raw.strip_prefix('#') {
                Some(after) => {
                    let end = after.find('#').map_or(value_raw.len(), |p| p + 1);
                    &value_raw[..end]
                }
                None => value_raw.split('#').next().unwrap_or(""),
            };
            out.terapkan_baris(key.trim(), value.trim(), line)?;
        }
        Ok(out)
    }

    /// Read and parse a file in one call.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ThemeFileError> {
        let text = std::fs::read_to_string(path.as_ref())?;
        Ok(Self::parse(&text)?)
    }

    /// True when the file changed nothing.
    pub fn is_empty(&self) -> bool {
        self.preset.is_none()
            && self.appearance.is_none()
            && self.colors.is_empty()
            && self.radii.is_empty()
            && self.corner_style.is_none()
            && self.space_unit.is_none()
            && self.controls.is_empty()
            && self.font_sizes.is_empty()
    }

    /// How many values will be written by [`ThemeOverrides::apply`].
    pub fn len(&self) -> usize {
        self.colors.len()
            + self.radii.len()
            + self.controls.len()
            + self.font_sizes.len()
            + usize::from(self.corner_style.is_some())
            + usize::from(self.space_unit.is_some())
    }

    /// The base preset the file asked for, if any.
    pub fn preset(&self) -> Option<Preset> {
        self.preset
    }

    /// The base appearance the file asked for, if any.
    pub fn appearance(&self) -> Option<Appearance> {
        self.appearance
    }

    /// Build the theme this file describes.
    ///
    /// `base` is what a file that says nothing at all resolves to — normally the
    /// theme the application already has, so a token file only needs to spell
    /// out what it wants to be different.
    pub fn apply(&self, base: Theme) -> Theme {
        let preset = self.preset.unwrap_or(base.preset);
        let appearance = self.appearance.unwrap_or(base.appearance);
        // Rebuilding from the preset rather than mutating `base` is what makes a
        // reload idempotent: editing `preset = tailwind` and back again returns
        // exactly the original theme instead of a hybrid.
        let mut theme = if preset == base.preset && appearance == base.appearance {
            base
        } else {
            Theme::new(preset, appearance)
        };

        for (token, color) in &self.colors {
            theme.color.set(*token, *color);
        }
        if let Some(style) = self.corner_style {
            theme.radius.style = style;
        }
        for (token, value) in &self.radii {
            let value = value.max(0.0);
            match token {
                RadiusToken::None => {}
                RadiusToken::Sm => theme.radius.sm = value,
                RadiusToken::Md => theme.radius.md = value,
                RadiusToken::Lg => theme.radius.lg = value,
                RadiusToken::Xl => theme.radius.xl = value,
                RadiusToken::Full => theme.radius.full = value,
            }
        }
        if let Some(unit) = self.space_unit {
            // A zero or negative unit would collapse the whole layout; a token
            // file is edited by hand, so it is guarded rather than trusted.
            theme.spacing.unit = unit.max(0.5);
        }
        for (token, value) in &self.controls {
            // A control cannot be shorter than a hairline; a token file is edited
            // by hand, so this is guarded rather than trusted.
            let value = value.max(1.0);
            match token {
                ControlToken::Sm => theme.control.sm = value,
                ControlToken::Md => theme.control.md = value,
                ControlToken::Lg => theme.control.lg = value,
                ControlToken::Row => theme.control.row = value,
                ControlToken::MenuRow => theme.control.menu_row = value,
            }
        }
        for (token, size) in &self.font_sizes {
            setel_ukuran_font(&mut theme.typography, *token, size.max(1.0));
        }
        theme
    }

    /// Write `theme` out in this format, completely.
    ///
    /// The output re-parses into the same theme, which the round-trip test in
    /// this module keeps true.
    pub fn dump(theme: &Theme) -> String {
        let mut out = String::new();
        out.push_str("# silka theme tokens\n");
        out.push_str("# Generated by ThemeOverrides::dump — edit and save; the\n");
        out.push_str("# preview app applies it without a restart (§9.1).\n\n");
        out.push_str(&format!("preset = {}\n", nama_preset(theme.preset)));
        out.push_str(&format!(
            "appearance = {}\n\n",
            nama_appearance(theme.appearance)
        ));

        out.push_str("# --- colors ---\n");
        for token in ColorToken::ALL {
            out.push_str(&format!(
                "color.{} = {}\n",
                token.name(),
                hex_dari_warna(theme.color.get(token))
            ));
        }

        out.push_str("\n# --- radius ---\n");
        out.push_str(&format!(
            "radius.style = {}\n",
            teks_corner_style(theme.radius.style)
        ));
        for token in RadiusToken::ALL {
            if matches!(token, RadiusToken::None) {
                continue;
            }
            out.push_str(&format!(
                "radius.{} = {}\n",
                token.name(),
                theme.radius.get(token)
            ));
        }

        out.push_str("\n# --- spacing ---\n");
        out.push_str(&format!("space.unit = {}\n", theme.spacing.unit));

        out.push_str("\n# --- control heights ---\n");
        out.push_str("# Visual height. The hit target is derived, never dumped:\n");
        out.push_str("# it is a rule (>= 44pt), not a value to tune.\n");
        for token in ControlToken::ALL {
            out.push_str(&format!(
                "control.{} = {}\n",
                token.name(),
                theme.control.get(token)
            ));
        }

        out.push_str("\n# --- typography ---\n");
        for token in FontToken::ALL {
            out.push_str(&format!(
                "font.{}.size = {}\n",
                token.name(),
                theme.typography.get(token).size
            ));
        }
        out
    }

    fn terapkan_baris(
        &mut self,
        key: &str,
        value: &str,
        line: usize,
    ) -> Result<(), TokenParseError> {
        let salah = |message: String| TokenParseError { line, message };
        match key {
            "preset" => {
                self.preset = Some(match value {
                    "cupertino" => Preset::Cupertino,
                    "tailwind" | "shadcn" => Preset::Tailwind,
                    other => {
                        return Err(salah(format!(
                            "unknown preset {other:?} — expected `cupertino` or `tailwind`"
                        )))
                    }
                });
            }
            "appearance" => {
                self.appearance = Some(match value {
                    "light" => Appearance::Light,
                    "dark" => Appearance::Dark,
                    other => {
                        return Err(salah(format!(
                            "unknown appearance {other:?} — expected `light` or `dark`"
                        )))
                    }
                });
            }
            "space.unit" => self.space_unit = Some(angka(value, line)?),
            "radius.style" => self.corner_style = Some(corner_style(value, line)?),
            _ => {
                if let Some(name) = key.strip_prefix("color.") {
                    let token = ColorToken::ALL
                        .into_iter()
                        .find(|t| t.name() == name)
                        .ok_or_else(|| salah(format!("unknown color token {name:?}")))?;
                    self.colors.push((token, warna(value, line)?));
                } else if let Some(name) = key.strip_prefix("radius.") {
                    let token = RadiusToken::ALL
                        .into_iter()
                        .find(|t| t.name() == name)
                        .ok_or_else(|| salah(format!("unknown radius token {name:?}")))?;
                    self.radii.push((token, angka(value, line)?));
                } else if let Some(name) = key.strip_prefix("control.") {
                    let token = ControlToken::ALL
                        .into_iter()
                        .find(|t| t.name() == name)
                        .ok_or_else(|| salah(format!("unknown control token {name:?}")))?;
                    self.controls.push((token, angka(value, line)?));
                } else if let Some(rest) = key.strip_prefix("font.") {
                    let Some(name) = rest.strip_suffix(".size") else {
                        return Err(salah(format!(
                            "only `font.<token>.size` is supported, found {key:?}"
                        )));
                    };
                    let token = FontToken::ALL
                        .into_iter()
                        .find(|t| t.name() == name)
                        .ok_or_else(|| salah(format!("unknown font token {name:?}")))?;
                    self.font_sizes.push((token, angka(value, line)?));
                } else {
                    return Err(salah(format!("unknown key {key:?}")));
                }
            }
        }
        Ok(())
    }
}

fn angka(value: &str, line: usize) -> Result<f32, TokenParseError> {
    value.parse::<f32>().map_err(|_| TokenParseError {
        line,
        message: format!("expected a number, found {value:?}"),
    })
}

fn warna(value: &str, line: usize) -> Result<Color, TokenParseError> {
    let digits = value.trim().trim_start_matches('#');
    let salah = || TokenParseError {
        line,
        message: format!("expected #RRGGBB or #RRGGBBAA, found {value:?}"),
    };
    match digits.len() {
        6 => {
            let rgb = u32::from_str_radix(digits, 16).map_err(|_| salah())?;
            Ok(Color::hex(rgb))
        }
        8 => {
            let rgba = u32::from_str_radix(digits, 16).map_err(|_| salah())?;
            Ok(Color::hexa(rgba))
        }
        _ => Err(salah()),
    }
}

fn corner_style(value: &str, line: usize) -> Result<CornerStyle, TokenParseError> {
    if value == "arc" {
        return Ok(CornerStyle::Arc);
    }
    if value == "squircle" {
        return Ok(CornerStyle::squircle());
    }
    if let Some(rest) = value.strip_prefix("squircle(") {
        if let Some(inner) = rest.strip_suffix(')') {
            let smoothing = angka(inner.trim(), line)?;
            return Ok(CornerStyle::Squircle {
                smoothing: smoothing.clamp(0.0, 1.0),
            });
        }
    }
    Err(TokenParseError {
        line,
        message: format!("expected `arc`, `squircle` or `squircle(0.0–1.0)`, found {value:?}"),
    })
}

fn setel_ukuran_font(t: &mut TypographyTokens, token: FontToken, size: f32) {
    let ubah = |style: &mut TypeStyle| {
        // The line height is stored as a multiple, so changing the size keeps
        // the rhythm of the scale instead of squashing it.
        style.size = size;
    };
    match token {
        FontToken::Caption2 => ubah(&mut t.caption2),
        FontToken::Caption1 => ubah(&mut t.caption1),
        FontToken::Footnote => ubah(&mut t.footnote),
        FontToken::Subheadline => ubah(&mut t.subheadline),
        FontToken::Callout => ubah(&mut t.callout),
        FontToken::Body => {
            ubah(&mut t.body);
            t.body_size = size;
        }
        FontToken::Headline => ubah(&mut t.headline),
        FontToken::Title3 => ubah(&mut t.title3),
        FontToken::Title2 => ubah(&mut t.title2),
        FontToken::Title1 => ubah(&mut t.title1),
        FontToken::LargeTitle => ubah(&mut t.large_title),
    }
}

fn nama_preset(preset: Preset) -> &'static str {
    match preset {
        Preset::Cupertino => "cupertino",
        Preset::Tailwind => "tailwind",
    }
}

fn nama_appearance(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    }
}

fn teks_corner_style(style: CornerStyle) -> String {
    match style {
        CornerStyle::Arc => String::from("arc"),
        CornerStyle::Squircle { smoothing } => format!("squircle({smoothing})"),
    }
}

fn hex_dari_warna(c: Color) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b, a) = (byte(c.r), byte(c.g), byte(c.b), byte(c.a));
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

// ---------------------------------------------------------------------------
// ThemeFile
// ---------------------------------------------------------------------------

/// A token file being watched by polling its modification time.
///
/// Polling rather than a file-system watcher, and on purpose: the framework
/// gains no dependency, the check is one `stat` per frame on a file that is
/// almost always in the OS cache, and it only happens in a development build.
/// A watcher (`notify`) is the right answer for a hundred files; this is one.
///
/// ```no_run
/// use silka_theme::dev::ThemeFile;
/// use silka_theme::Theme;
///
/// let mut file = ThemeFile::new("design/tokens.silka");
/// let mut theme = Theme::default();
///
/// // Once per frame, in a dev build. `None` means "nothing changed", which is
/// // the answer 99.99% of the time and costs one `stat`.
/// if let Some(result) = file.poll() {
///     match result {
///         Ok(overrides) => theme = overrides.apply(theme),
///         // A half-saved file parses badly for a few milliseconds. Keeping the
///         // last good theme and showing the message beats flashing a broken UI.
///         Err(e) => eprintln!("tokens: {e}"),
///     }
/// }
/// ```
#[derive(Debug)]
pub struct ThemeFile {
    path: PathBuf,
    stamp: Option<SystemTime>,
    /// True until the first poll, so the first call always loads.
    fresh: bool,
}

impl ThemeFile {
    /// Watch `path`. Nothing is read until the first [`ThemeFile::poll`].
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            stamp: None,
            fresh: true,
        }
    }

    /// The file being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the file now, regardless of whether it changed.
    pub fn load(&mut self) -> Result<ThemeOverrides, ThemeFileError> {
        self.stamp = self.mtime();
        self.fresh = false;
        ThemeOverrides::from_path(&self.path)
    }

    /// Return the overrides **only when the file changed** since the last poll.
    ///
    /// The first call always reads (there is nothing to compare against yet). A
    /// missing file is not an error while polling — a designer deleting the file
    /// should not take the application down — so it yields `None` and the
    /// application keeps the theme it has.
    pub fn poll(&mut self) -> Option<Result<ThemeOverrides, ThemeFileError>> {
        let now = self.mtime();
        if now.is_none() {
            self.stamp = None;
            self.fresh = false;
            return None;
        }
        if !self.fresh && now == self.stamp {
            return None;
        }
        Some(self.load())
    }

    /// Write a complete token file for `theme`, creating the parent directory.
    ///
    /// This is how a project gets its first token file: dump the theme it
    /// already uses, then start editing.
    pub fn write_template(path: impl AsRef<Path>, theme: &Theme) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, ThemeOverrides::dump(theme))
    }

    fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path)
            .ok()
            .and_then(|m| m.modified().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baris_kosong_dan_komentar_dilewati() {
        let o = ThemeOverrides::parse("\n# komentar\n   \n").unwrap();
        assert!(o.is_empty());
        assert_eq!(o.len(), 0);
    }

    #[test]
    fn warna_rgb_dan_rgba() {
        let o = ThemeOverrides::parse("color.accent = #0A84FF\ncolor.scrim = #00000066").unwrap();
        let t = o.apply(Theme::default());
        assert_eq!(t.color.get(ColorToken::Accent), Color::hex(0x0A84FF));
        assert_eq!(t.color.get(ColorToken::Scrim), Color::hexa(0x00000066));
    }

    #[test]
    fn komentar_setelah_nilai_warna_tidak_memakan_warnanya() {
        let o = ThemeOverrides::parse("color.accent = #0A84FF # biru sistem").unwrap();
        let t = o.apply(Theme::default());
        assert_eq!(t.color.get(ColorToken::Accent), Color::hex(0x0A84FF));
    }

    #[test]
    fn preset_dan_appearance_menjadi_basis() {
        let o = ThemeOverrides::parse("preset = tailwind\nappearance = dark").unwrap();
        let t = o.apply(Theme::cupertino(Appearance::Light));
        assert_eq!(t.preset, Preset::Tailwind);
        assert_eq!(t.appearance, Appearance::Dark);
        // The whole palette came from the new preset, not just the two lines.
        assert_eq!(t.color, Theme::tailwind(Appearance::Dark).color);
    }

    #[test]
    fn menerapkan_dua_kali_hasilnya_sama() {
        let o = ThemeOverrides::parse("preset = tailwind\ncolor.accent = #FF0000").unwrap();
        let sekali = o.apply(Theme::default());
        let dua_kali = o.apply(o.apply(Theme::default()));
        assert_eq!(sekali.color, dua_kali.color);
        assert_eq!(sekali.radius, dua_kali.radius);
    }

    #[test]
    fn radius_dan_gaya_sudut() {
        let o = ThemeOverrides::parse("radius.style = squircle(0.4)\nradius.md = 9.5").unwrap();
        let t = o.apply(Theme::default());
        assert_eq!(t.radius.md, 9.5);
        assert_eq!(t.radius.style, CornerStyle::Squircle { smoothing: 0.4 });

        let arc = ThemeOverrides::parse("radius.style = arc")
            .unwrap()
            .apply(Theme::default());
        assert_eq!(arc.radius.style, CornerStyle::Arc);
    }

    #[test]
    fn unit_spasi_tidak_boleh_nol() {
        let t = ThemeOverrides::parse("space.unit = 0")
            .unwrap()
            .apply(Theme::default());
        assert!(t.spacing.unit >= 0.5, "unit 0 akan mengempiskan seluruh UI");
    }

    #[test]
    fn ukuran_font_body_ikut_body_size() {
        let t = ThemeOverrides::parse("font.body.size = 15")
            .unwrap()
            .apply(Theme::default());
        assert_eq!(t.typography.get(FontToken::Body).size, 15.0);
        assert_eq!(t.typography.body_size, 15.0);
    }

    #[test]
    fn pesan_salah_menyebut_nomor_baris() {
        let e = ThemeOverrides::parse("preset = cupertino\ncolor.ngawur = #FFFFFF").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.to_string().contains("line 2"));
        assert!(e.message.contains("ngawur"));

        let e = ThemeOverrides::parse("tanpa tanda sama dengan").unwrap_err();
        assert_eq!(e.line, 1);

        let e = ThemeOverrides::parse("radius.md = biru").unwrap_err();
        assert!(e.message.contains("number"));

        let e = ThemeOverrides::parse("color.accent = 0A84FF00FF").unwrap_err();
        assert!(e.message.contains("RRGGBB"));

        let e = ThemeOverrides::parse("preset = bootstrap").unwrap_err();
        assert!(e.message.contains("cupertino"));
    }

    #[test]
    fn dump_bolak_balik_untuk_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let asli = Theme::new(preset, appearance);
                let teks = ThemeOverrides::dump(&asli);
                let pulang = ThemeOverrides::parse(&teks)
                    .expect("dump harus bisa dibaca kembali")
                    .apply(Theme::default());
                assert_eq!(pulang.preset, asli.preset);
                assert_eq!(pulang.appearance, asli.appearance);
                assert_eq!(pulang.radius, asli.radius);
                assert_eq!(pulang.spacing, asli.spacing);
                for token in FontToken::ALL {
                    assert_eq!(
                        pulang.typography.get(token).size,
                        asli.typography.get(token).size,
                        "{preset:?}/{appearance:?} {}",
                        token.name()
                    );
                }
                // Colors are compared through the file rather than as floats:
                // the format is 8 bits per channel, so "round-trips" means the
                // *text* comes back identical, not that a f32 survived being
                // quantised (`0.6` → 153 → `0.6`).
                assert_eq!(ThemeOverrides::dump(&pulang), teks);
            }
        }
    }

    #[test]
    fn berkas_hilang_bukan_kesalahan_saat_polling() {
        let mut f = ThemeFile::new("tidak/ada/berkas.silka");
        assert!(
            f.poll().is_none(),
            "berkas hilang tidak boleh mematikan app"
        );
        assert!(f.poll().is_none());
    }

    #[test]
    fn polling_membaca_sekali_lalu_diam() {
        let dir = std::env::temp_dir().join(format!("silka-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.silka");
        std::fs::write(&path, "color.accent = #FF0000\n").unwrap();

        let mut file = ThemeFile::new(&path);
        let pertama = file.poll().expect("panggilan pertama selalu membaca");
        assert_eq!(
            pertama.unwrap().apply(Theme::default()).color.accent,
            Color::hex(0xFF0000)
        );
        assert!(
            file.poll().is_none(),
            "tanpa perubahan tidak ada pekerjaan sama sekali"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn template_bisa_ditulis_dan_dibaca() {
        let dir = std::env::temp_dir().join(format!("silka-tpl-{}", std::process::id()));
        let path = dir.join("nested/tokens.silka");
        let theme = Theme::tailwind(Appearance::Dark);
        ThemeFile::write_template(&path, &theme).unwrap();

        let dibaca = ThemeOverrides::from_path(&path)
            .unwrap()
            .apply(Theme::default());
        assert_eq!(ThemeOverrides::dump(&dibaca), ThemeOverrides::dump(&theme));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
