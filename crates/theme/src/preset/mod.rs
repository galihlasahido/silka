//! The two first-party presets (§2.7).
//!
//! A preset is the **only** place semantic tokens meet numbers. Widgets never
//! name this module; they name [`crate::Theme`], and a theme is built from
//! `(Preset, Appearance)`.
//!
//! A third preset (a custom brand) needs no new file: start from one of these
//! presets and swap its tokens via `Theme::with_colors`/`with_radius`/
//! `with_typography` (§2.7 — "just fill in the tokens").

pub mod cupertino;
pub mod tailwind;
