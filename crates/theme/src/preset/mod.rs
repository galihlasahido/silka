//! The two first-party presets (§2.7).
//!
//! A preset is the **only** place semantic tokens meet numbers. Widgets never
//! name this module; they name [`crate::Theme`], and a theme is built from
//! `(Preset, Appearance)`.
//!
//! A third preset (a custom brand) needs no new file: start from one of these
//! presets and swap its tokens via `Theme::with_colors`/`with_radius`/
//! `with_typography` (§2.7 — "just fill in the tokens").
//!
//! ```
//! use silka_paint::{Color, CornerStyle};
//! use silka_theme::{Appearance, ColorToken, Preset, RadiusToken, Theme};
//!
//! // The four first-party combinations, all built the same way.
//! for preset in Preset::ALL {
//!     for appearance in [Appearance::Light, Appearance::Dark] {
//!         let t = Theme::new(preset, appearance);
//!         assert_eq!(t.appearance, appearance);
//!     }
//! }
//!
//! // What actually differs is the numbers behind identical role names…
//! let hig = Theme::cupertino(Appearance::Dark);
//! let shadcn = Theme::tailwind(Appearance::Dark);
//! assert_eq!(hig.corners_of(RadiusToken::Lg).style, CornerStyle::squircle());
//! assert_eq!(shadcn.corners_of(RadiusToken::Lg).style, CornerStyle::Arc);
//!
//! // …and a brand preset is a first-party one with tokens overwritten, not a
//! // third file to maintain.
//! let brand = hig.map_colors(|token, color| match token {
//!     ColorToken::Accent => Color::hex(0x7C3AED),
//!     _ => color,
//! });
//! assert_eq!(brand.color.accent, Color::hex(0x7C3AED));
//! assert_eq!(brand.color.background, hig.color.background); // the rest is untouched
//! ```

pub mod cupertino;
pub mod tailwind;
