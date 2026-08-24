//! Static, dummy data — and the one piece of real logic this application has:
//! [`validate_email`].
//!
//! Everything else here is just enough shape to make the form feel like it
//! belongs to a real product, the same rule `silka-dashboard`'s `data` module
//! follows.

/// The signed-in person's display name, seeded into the form on launch.
pub const SEED_NAME: &str = "Dian Permata";
/// …and their email.
pub const SEED_EMAIL: &str = "dian.permata@example.com";

/// The languages the "Language" select offers.
pub const LANGUAGES: [&str; 3] = ["English", "Bahasa Indonesia", "日本語"];

/// The accent colours the "Accent colour" picker offers — a small, deliberate
/// set rather than a full spectrum, the same choice `silka-gallery`'s
/// "the application palette, by name" case makes.
pub const ACCENT_NAMES: [&str; 6] = ["Blue", "Purple", "Pink", "Orange", "Green", "Teal"];

/// The smallest and largest preview font size the "Font size" stepper allows.
pub const FONT_SIZE_RANGE: (f32, f32) = (12.0, 24.0);
/// The smallest and largest session timeout, in minutes.
pub const SESSION_TIMEOUT_RANGE: (f32, f32) = (5.0, 120.0);

/// A trusted device, as the "Security" tab lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Device {
    /// What the device calls itself.
    pub name: &'static str,
    /// Where it last signed in from.
    pub location: &'static str,
}

/// The devices seeded into the list on launch.
pub const SEED_DEVICES: [Device; 3] = [
    Device {
        name: "MacBook Pro",
        location: "Jakarta, ID",
    },
    Device {
        name: "iPhone 15",
        location: "Jakarta, ID",
    },
    Device {
        name: "Chrome on Windows",
        location: "Surabaya, ID",
    },
];

/// Why an email address is unfit to save, if it is.
///
/// Deliberately not a real RFC 5322 validator — a settings form's job is to
/// catch a typo, not to reimplement the mail spec — but it is a **function**
/// rather than a literal `contains('@')` scattered at the call site, so the
/// rule is stated once and testable on its own.
pub fn validate_email(value: &str) -> Option<&'static str> {
    if value.trim().is_empty() {
        return Some("Email is required");
    }
    let Some((local, domain)) = value.split_once('@') else {
        return Some("Missing the @");
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Some("Not a valid email address");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_address_passes() {
        assert_eq!(validate_email("dian.permata@example.com"), None);
    }

    #[test]
    fn an_empty_address_asks_to_be_filled_in() {
        assert_eq!(validate_email(""), Some("Email is required"));
        assert_eq!(validate_email("   "), Some("Email is required"));
    }

    #[test]
    fn a_missing_at_sign_is_named_specifically() {
        assert_eq!(validate_email("dian.example.com"), Some("Missing the @"));
    }

    #[test]
    fn a_domain_without_a_dot_is_rejected() {
        assert_eq!(
            validate_email("dian@example"),
            Some("Not a valid email address")
        );
    }

    #[test]
    fn an_empty_local_part_is_rejected() {
        assert_eq!(
            validate_email("@example.com"),
            Some("Not a valid email address")
        );
    }
}
