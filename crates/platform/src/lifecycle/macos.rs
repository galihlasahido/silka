//! Reading the macOS lifecycle settings (INTEGRASI-NATIVE §6, §8).
//!
//! Everything here goes through `NSUserDefaults`, which is the layer the
//! System Settings panes actually write to:
//!
//! | Setting | Domain | Key |
//! |---|---|---|
//! | Accent color | global | `AppleAccentColor` (absent = "Multicolor") |
//! | Selection color | global | `AppleHighlightColor` (`"r g b Name"`) |
//! | Reduce motion | `NSWorkspace.accessibilityDisplayShouldReduceMotion` | — |
//! | Reduce transparency | `NSWorkspace.accessibilityDisplayShouldReduceTransparency` | — |
//!
//! Why the accent goes through user defaults while the accessibility flags go
//! through AppKit: `NSColor::controlAccentColor` is a **catalog** color that
//! only turns into components inside a drawing context with the right
//! appearance, whereas the defaults key is plain data naming the light/dark
//! pair explicitly, mappable by a pure function that is unit-tested on every
//! platform ([`super::mac_defaults`]). The accessibility flags have no such
//! problem: `NSWorkspace` answers them directly, and — unlike reading
//! `com.apple.universalaccess` by hand — it keeps working inside the App Store
//! sandbox.
//!
//! One deliberate limitation:
//!
//! - **Nothing here observes.** There is no distributed-notification observer,
//!   because an observer that fires on a background thread would have to be
//!   funnelled back into the event loop for no gain: the shell re-reads on the
//!   events the OS already delivers (theme change, window focus), and a
//!   setting changed in System Settings therefore lands the moment the user
//!   comes back to the window.

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSUserDefaults};
use silka_core::animation::Motion;
use silka_theme::{Appearance, Transparency};

use super::mac_defaults::{accent_color, parse_highlight_color, KEY_ACCENT, KEY_HIGHLIGHT};
use super::SystemSettings;

/// Read every §6 setting macOS exposes.
pub fn read(appearance: Appearance) -> SystemSettings {
    SystemSettings {
        accent: accent(appearance),
        selection: selection(),
        motion: Motion::from_reduced(reduces_motion()),
        transparency: Transparency::from_reduced(reduces_transparency()),
    }
}

/// The user's accent color, or `None` when it is left on "Multicolor".
///
/// The presence check is not redundant: `integerForKey:` reports `0` both for
/// "absent" and for "red", and treating a missing key as red would repaint
/// every default install.
fn accent(appearance: Appearance) -> Option<silka_paint::Color> {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(KEY_ACCENT);
    defaults.objectForKey(&key)?;
    accent_color(defaults.integerForKey(&key) as i64, appearance)
}

/// The user's text-selection color, when they chose one.
fn selection() -> Option<silka_paint::Color> {
    let defaults = NSUserDefaults::standardUserDefaults();
    let raw = defaults.stringForKey(&NSString::from_str(KEY_HIGHLIGHT))?;
    parse_highlight_color(&raw.to_string())
}

/// "Reduce motion", straight from AppKit.
///
/// Read fresh on every call rather than cached: the user can turn it on while
/// the application is running, and a cached `false` would keep the bounce for
/// the rest of the session.
fn reduces_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// "Reduce transparency", straight from AppKit.
///
/// The same source [`crate::titlebar::system_reduces_transparency`] uses, on
/// purpose: a window whose vibrancy is switched off but whose tokens stayed
/// translucent would be worse than either answer on its own.
fn reduces_transparency() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membaca_setelan_mac_tidak_pernah_panik() {
        // Whatever this machine has configured — accent on Multicolor, no
        // accessibility domain at all — reading it must produce a value.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let s = read(appearance);
            if let Some(c) = s.accent {
                assert_eq!(c.a, 1.0, "aksen OS selalu buram");
            }
        }
    }

    #[test]
    fn bendera_aksesibilitas_sepakat_dengan_yang_dipakai_vibrancy() {
        // Two readers of one setting is one too many *if they can disagree*: a
        // window whose blur is off but whose tokens are still translucent looks
        // broken in a way nobody would think to test for.
        assert_eq!(
            reduces_transparency(),
            crate::titlebar::system_reduces_transparency()
        );
    }

    #[test]
    fn membaca_dua_kali_memberi_jawaban_yang_sama() {
        // No caching of our own: two reads in a row see the same OS state.
        assert_eq!(read(Appearance::Dark), read(Appearance::Dark));
    }
}
