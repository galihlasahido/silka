//! Version numbers, and the only ordering an updater is allowed to use.
//!
//! An update is a comparison before it is anything else, and the comparison is
//! where naive updaters go wrong: string ordering puts `1.4.0-rc.10` *before*
//! `1.4.0-rc.2`, and puts `1.10.0` before `1.9.0`. Both mistakes ship an older
//! build to a user who already has the newer one, which is the one failure mode
//! an updater must not have.
//!
//! ```
//! use silka_dist::version::Version;
//!
//! let rc2 = Version::parse("1.4.0-rc.2").unwrap();
//! let rc10 = Version::parse("1.4.0-rc.10").unwrap();
//! assert!(rc2 < rc10, "numeric pre-release fields compare as numbers");
//!
//! // A release always outranks its own pre-releases.
//! assert!(Version::parse("1.4.0").unwrap() > rc10);
//! ```
//!
//! # What this is not
//!
//! It is a **subset** of semver, and the subset is chosen by what appears in a
//! release feed rather than by what the specification allows:
//!
//! - Build metadata (`+abc123`) parses and is then ignored in comparisons,
//!   exactly as semver requires — two builds of the same version are the same
//!   version, and offering one to the other is offering nothing.
//! - Missing components are zero, so `"12.0"` and `"10"` parse. This is *not*
//!   semver, and it is here because operating systems report versions that way:
//!   `minimum_os.macos` in a feed is `"12.0"`, never `"12.0.0"`.
//! - Leading zeros in a numeric field are rejected. `1.01.0` and `1.1.0` would
//!   otherwise be two spellings of one version, and a feed with both in it is a
//!   feed nobody can debug.

use std::cmp::Ordering;
use std::fmt;

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// A parsed version number, ordered the way an updater needs it ordered.
///
/// Cheap to clone, comparable, and round-trips through [`fmt::Display`]:
/// whatever a feed said is what this prints back, minus build metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Vec<PreField>,
}

/// One dot-separated field of a pre-release tag.
///
/// Kept private: the distinction between "numeric" and "textual" matters only
/// to the ordering, and exposing it would invite code that branches on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PreField {
    Num(u64),
    Text(String),
}

impl Version {
    /// The zero version — older than everything that parses.
    ///
    /// Useful as the "current version" of an install that has no idea what it
    /// is, which happens exactly once: the first run after a sideload.
    pub const ZERO: Version = Version {
        major: 0,
        minor: 0,
        patch: 0,
        pre: Vec::new(),
    };

    /// Build a release version directly, without going through a string.
    pub const fn new(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
            pre: Vec::new(),
        }
    }

    /// Parse `"1.4.0"`, `"v1.4.0"`, `"1.4.0-rc.2"`, `"1.4.0+9e75a29"`, `"12.0"`.
    ///
    /// ```
    /// use silka_dist::version::Version;
    ///
    /// assert_eq!(Version::parse("v1.4.0+9e75a29").unwrap().to_string(), "1.4.0");
    /// assert_eq!(Version::parse("12.0").unwrap(), Version::new(12, 0, 0));
    /// assert!(Version::parse("1.01.0").is_err(), "leading zeros are two spellings of one version");
    /// ```
    pub fn parse(text: &str) -> Result<Version, VersionError> {
        let trimmed = text.trim();
        let trimmed = trimmed.strip_prefix('v').unwrap_or(trimmed);
        if trimmed.is_empty() {
            return Err(VersionError::Empty);
        }

        // Build metadata never participates in ordering, so it is dropped as
        // early as possible rather than carried around and remembered about.
        let without_build = match trimmed.split_once('+') {
            Some((head, _build)) => head,
            None => trimmed,
        };

        let (core, pre_text) = match without_build.split_once('-') {
            Some((head, tail)) => (head, Some(tail)),
            None => (without_build, None),
        };

        let mut numbers = [0u64; 3];
        let mut seen = 0usize;
        for part in core.split('.') {
            if seen == 3 {
                return Err(VersionError::TooManyParts);
            }
            numbers[seen] = parse_number(part)?;
            seen += 1;
        }
        if seen == 0 {
            return Err(VersionError::Empty);
        }

        let mut pre = Vec::new();
        if let Some(tag) = pre_text {
            if tag.is_empty() {
                return Err(VersionError::EmptyPreField);
            }
            for field in tag.split('.') {
                pre.push(parse_pre_field(field)?);
            }
        }

        Ok(Version {
            major: numbers[0],
            minor: numbers[1],
            patch: numbers[2],
            pre,
        })
    }

    /// The first component.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// The second component.
    pub fn minor(&self) -> u64 {
        self.minor
    }

    /// The third component.
    pub fn patch(&self) -> u64 {
        self.patch
    }

    /// Whether this version carries a pre-release tag.
    ///
    /// The updater uses it for one decision only: an install on the stable
    /// channel is never offered a build that says it is not finished.
    pub fn is_pre_release(&self) -> bool {
        !self.pre.is_empty()
    }

    /// The pre-release tag as written, without its leading `-`.
    ///
    /// `None` for a release version.
    pub fn pre_release(&self) -> Option<String> {
        if self.pre.is_empty() {
            return None;
        }
        let mut out = String::new();
        for (index, field) in self.pre.iter().enumerate() {
            if index > 0 {
                out.push('.');
            }
            match field {
                PreField::Num(n) => out.push_str(&n.to_string()),
                PreField::Text(t) => out.push_str(t),
            }
        }
        Some(out)
    }
}

fn parse_number(part: &str) -> Result<u64, VersionError> {
    if part.is_empty() {
        return Err(VersionError::MissingPart);
    }
    if part.len() > 1 && part.starts_with('0') {
        return Err(VersionError::LeadingZero);
    }
    part.parse::<u64>().map_err(|_| VersionError::NotANumber)
}

fn parse_pre_field(field: &str) -> Result<PreField, VersionError> {
    if field.is_empty() {
        return Err(VersionError::EmptyPreField);
    }
    let numeric = field.bytes().all(|b| b.is_ascii_digit());
    if numeric {
        if field.len() > 1 && field.starts_with('0') {
            return Err(VersionError::LeadingZero);
        }
        let value = field.parse::<u64>().map_err(|_| VersionError::NotANumber)?;
        return Ok(PreField::Num(value));
    }
    let allowed = field
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !allowed {
        return Err(VersionError::BadCharacter);
    }
    Ok(PreField::Text(field.to_string()))
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(tag) = self.pre_release() {
            write!(f, "-{tag}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        let numeric = self
            .major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch));
        if numeric != Ordering::Equal {
            return numeric;
        }

        // Same numbers: a release outranks every pre-release of itself. This is
        // the rule that keeps `1.4.0` from being offered to nobody because
        // `1.4.0-rc.9` sorted above it.
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => cmp_pre(&self.pre, &other.pre),
        }
    }
}

fn cmp_pre(left: &[PreField], right: &[PreField]) -> Ordering {
    let shared = left.len().min(right.len());
    for index in 0..shared {
        let ordering = match (&left[index], &right[index]) {
            (PreField::Num(a), PreField::Num(b)) => a.cmp(b),
            // Semver: numeric fields always rank below textual ones.
            (PreField::Num(_), PreField::Text(_)) => Ordering::Less,
            (PreField::Text(_), PreField::Num(_)) => Ordering::Greater,
            (PreField::Text(a), PreField::Text(b)) => a.cmp(b),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    // Everything shared is equal, so the version with more fields is more
    // specific and therefore later: `rc.1` precedes `rc.1.1`.
    left.len().cmp(&right.len())
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a version string could not be read.
///
/// Every variant names the *shape* problem rather than echoing the input: these
/// end up in a log line next to the string that produced them, and repeating it
/// twice helps nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionError {
    /// The string was empty, or `v` on its own.
    Empty,
    /// A dot-separated component was missing between two dots.
    MissingPart,
    /// More than three dot-separated numeric components.
    TooManyParts,
    /// A component was not a decimal number, or did not fit in a `u64`.
    NotANumber,
    /// A numeric component had a leading zero — two spellings of one version.
    LeadingZero,
    /// The pre-release tag was empty, or had an empty field between two dots.
    EmptyPreField,
    /// The pre-release tag contained something other than `[0-9A-Za-z-]`.
    BadCharacter,
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            VersionError::Empty => "version is empty",
            VersionError::MissingPart => "version has an empty component",
            VersionError::TooManyParts => "version has more than three components",
            VersionError::NotANumber => "version component is not a number",
            VersionError::LeadingZero => "version component has a leading zero",
            VersionError::EmptyPreField => "pre-release tag has an empty field",
            VersionError::BadCharacter => "pre-release tag has a character outside [0-9A-Za-z-]",
        };
        f.write_str(text)
    }
}

impl std::error::Error for VersionError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect("versi harus terbaca")
    }

    #[test]
    fn komponen_terbaca() {
        let parsed = v("1.4.7");
        assert_eq!(parsed.major(), 1);
        assert_eq!(parsed.minor(), 4);
        assert_eq!(parsed.patch(), 7);
        assert!(!parsed.is_pre_release());
    }

    #[test]
    fn awalan_v_dan_metadata_build_diabaikan() {
        assert_eq!(v("v1.4.0"), v("1.4.0"));
        assert_eq!(v("1.4.0+9e75a29"), v("1.4.0"));
        assert_eq!(v("1.4.0+9e75a29").to_string(), "1.4.0");
    }

    #[test]
    fn komponen_yang_hilang_jadi_nol() {
        assert_eq!(v("12.0"), Version::new(12, 0, 0));
        assert_eq!(v("10"), Version::new(10, 0, 0));
    }

    #[test]
    fn urutan_numerik_bukan_leksikografis() {
        assert!(v("1.9.0") < v("1.10.0"));
        assert!(v("1.4.9") < v("1.4.10"));
        assert!(v("2.0.0") > v("1.999.999"));
    }

    #[test]
    fn rc2_sebelum_rc10() {
        assert!(v("1.4.0-rc.2") < v("1.4.0-rc.10"));
    }

    #[test]
    fn rilis_mengalahkan_pra_rilisnya_sendiri() {
        assert!(v("1.4.0") > v("1.4.0-rc.10"));
        assert!(v("1.4.0") > v("1.4.0-beta"));
        assert!(v("1.4.0-rc.1") < v("1.4.0"));
    }

    #[test]
    fn bidang_numerik_di_bawah_bidang_teks() {
        assert!(v("1.0.0-1") < v("1.0.0-alpha"));
    }

    #[test]
    fn tag_lebih_panjang_lebih_baru_saat_awalan_sama() {
        assert!(v("1.0.0-rc.1") < v("1.0.0-rc.1.1"));
    }

    #[test]
    fn urutan_semver_penuh() {
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for window in ordered.windows(2) {
            assert!(
                v(window[0]) < v(window[1]),
                "{} harus di bawah {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn zero_lebih_tua_dari_apa_pun() {
        assert!(Version::ZERO < v("0.0.1"));
        assert!(Version::ZERO < v("0.1.0-alpha"));
        assert_eq!(Version::ZERO, v("0.0.0"));
    }

    #[test]
    fn cetak_bolak_balik() {
        for text in ["1.4.0", "1.4.0-rc.2", "0.0.1-alpha.1.2", "12.0.0"] {
            assert_eq!(v(text).to_string(), text);
        }
    }

    #[test]
    fn tag_pra_rilis_dikembalikan_apa_adanya() {
        assert_eq!(v("1.4.0-rc.2").pre_release().as_deref(), Some("rc.2"));
        assert_eq!(v("1.4.0").pre_release(), None);
    }

    #[test]
    fn bentuk_yang_ditolak() {
        assert_eq!(Version::parse(""), Err(VersionError::Empty));
        assert_eq!(Version::parse("v"), Err(VersionError::Empty));
        assert_eq!(Version::parse("1..0"), Err(VersionError::MissingPart));
        assert_eq!(Version::parse("1.2.3.4"), Err(VersionError::TooManyParts));
        assert_eq!(Version::parse("1.x.0"), Err(VersionError::NotANumber));
        assert_eq!(Version::parse("1.01.0"), Err(VersionError::LeadingZero));
        assert_eq!(Version::parse("1.0.0-"), Err(VersionError::EmptyPreField));
        assert_eq!(
            Version::parse("1.0.0-a..b"),
            Err(VersionError::EmptyPreField)
        );
        assert_eq!(
            Version::parse("1.0.0-rc_2"),
            Err(VersionError::BadCharacter)
        );
        assert_eq!(Version::parse("1.0.0-01"), Err(VersionError::LeadingZero));
    }

    #[test]
    fn nol_tunggal_bukan_leading_zero() {
        assert_eq!(v("0.0.0"), Version::new(0, 0, 0));
        assert_eq!(v("1.0.0-0").to_string(), "1.0.0-0");
    }

    #[test]
    fn galat_punya_pesan() {
        assert!(VersionError::LeadingZero
            .to_string()
            .contains("leading zero"));
    }
}
