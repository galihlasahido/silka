//! Dua preset first-party (§2.7).
//!
//! Preset adalah **satu-satunya** tempat token semantik bertemu angka. Widget
//! tidak pernah menyebut modul ini; ia menyebut [`crate::Theme`], dan theme
//! dibangun dari `(Preset, Appearance)`.
//!
//! Preset ketiga (brand kustom) tidak perlu file baru: mulai dari salah satu
//! preset ini lalu ganti tokennya lewat `Theme::with_colors`/`with_radius`/
//! `with_typography` (§2.7 — "tinggal isi token").

pub mod cupertino;
pub mod tailwind;
