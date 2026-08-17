//! Credential storage and biometrics (INTEGRASI-NATIVE §5).
//!
//! Two related things an application needs before it can hold a token: somewhere
//! to keep it that is not a file in the home directory, and a way to ask the
//! human in front of the machine to prove they are still there.
//!
//! ## Where a secret goes
//!
//! [`Credential`] is the OS credential store — Keychain on macOS, Credential
//! Manager on Windows — reached through `keyring`, which is confined to this
//! module the way `arboard` is confined to [`mod@crate::clipboard`] (§3.2).
//!
//! ```no_run
//! use silka_platform::credential::credential;
//!
//! let token = credential("com.example.editor", "sync-token");
//! token.set_password("s3cr3t")?;
//! assert_eq!(token.password()?, "s3cr3t");
//! token.delete()?;
//! # Ok::<(), silka_platform::credential::CredentialError>(())
//! ```
//!
//! **Linux is deliberately absent.** The Secret Service backend needs either a
//! system `libdbus` at build time or the kernel keyring (which does not survive
//! a reboot, so it is not credential storage at all), and neither is something
//! to enable behind an application's back. `keyring` is therefore not even a
//! dependency on Linux — it defaults to an **in-memory mock** there, and an
//! application that thought it had saved a token to the login keyring and had
//! actually saved it to a `HashMap` is a worse outcome than a clear
//! [`CredentialError::Unsupported`]. [`is_supported`] answers the question
//! before anything is stored.
//!
//! ## Proving the human is still there
//!
//! [`BiometricPrompt`] is Touch ID / Face ID / Windows Hello. The vocabulary is
//! here; the backends are not, and the module says which API each one needs
//! rather than pretending. What **is** worth stating up front, and is encoded in
//! the API, is that biometrics is an *authorisation* gesture and never a
//! *storage* mechanism: it returns "the user is present", not a key.

use core::fmt;

/// Why a credential operation did not happen.
///
/// [`CredentialError::NotFound`] is an ordinary state, not an exception: the
/// first run of an application has no saved token, and that is the normal path
/// through the code rather than an error to report.
///
/// ```no_run
/// use silka_platform::credential::{credential, CredentialError};
///
/// match credential("com.example.editor", "sync-token").password() {
///     Ok(token) => println!("signed in with {} characters", token.len()),
///     Err(CredentialError::NotFound) => println!("first run: ask the user to sign in"),
///     Err(CredentialError::Unsupported(_)) => println!("no credential store here"),
///     Err(e) => println!("credential store unavailable: {e}"),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialError {
    /// Nothing is stored under this service and account. The first run.
    NotFound,
    /// The store exists but is locked — a keychain the user has not unlocked.
    /// Worth prompting about, and worth retrying afterwards.
    Locked,
    /// The stored value is not valid UTF-8, so it is not a password.
    /// [`Credential::secret`] can still read it as bytes.
    NotText,
    /// The service or account name is empty. Every backend keys on the pair,
    /// and an empty half means every caller shares one slot.
    EmptyName,
    /// There is no credential store on this build. The message says why.
    Unsupported(String),
    /// Anything else the OS reported.
    Os(String),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::NotFound => write!(f, "nothing stored under that name"),
            CredentialError::Locked => write!(f, "the credential store is locked"),
            CredentialError::NotText => write!(f, "the stored value is not text"),
            CredentialError::EmptyName => {
                write!(f, "a credential needs both a service and an account name")
            }
            CredentialError::Unsupported(m) => write!(f, "no credential store here: {m}"),
            CredentialError::Os(m) => write!(f, "the credential store failed: {m}"),
        }
    }
}

impl std::error::Error for CredentialError {}

/// Whether this build can reach a real OS credential store.
///
/// False on Linux — see the module documentation for why that is a decision
/// rather than an omission.
///
/// ```
/// use silka_platform::credential::is_supported;
///
/// if !is_supported() {
///     // Ask for the token every run rather than pretending it was saved.
/// }
/// ```
pub const fn is_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// One slot in the OS credential store, named by a service and an account.
///
/// The pair is the key on every platform: the service is the application or
/// the remote system ("com.example.editor", "api.example.com") and the account
/// is who the secret belongs to. Both halves matter — an application that
/// stores every user's token under one account name overwrites them in turn.
///
/// A plain value: constructing one touches nothing, and every method is a
/// separate trip to the OS.
///
/// ```
/// use silka_platform::credential::{credential, CredentialError};
///
/// let entry = credential("com.example.editor", "ana@example.com");
/// assert_eq!(entry.service(), "com.example.editor");
/// assert_eq!(entry.account(), "ana@example.com");
///
/// // An empty half is refused before the OS is asked: it would silently make
/// // every caller share one slot.
/// assert_eq!(credential("", "ana").check(), Err(CredentialError::EmptyName));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Credential {
    service: String,
    account: String,
}

/// Name a slot in the OS credential store.
pub fn credential(service: impl Into<String>, account: impl Into<String>) -> Credential {
    Credential {
        service: service.into(),
        account: account.into(),
    }
}

impl Credential {
    /// The service — the application or remote system this secret belongs to.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The account — who the secret belongs to.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Whether the name is usable.
    ///
    /// Checked separately from the storage calls so an application can assert
    /// it in a test that never touches a keychain.
    pub fn check(&self) -> Result<(), CredentialError> {
        if self.service.trim().is_empty() || self.account.trim().is_empty() {
            return Err(CredentialError::EmptyName);
        }
        Ok(())
    }

    /// Store a password.
    pub fn set_password(&self, password: &str) -> Result<(), CredentialError> {
        self.check()?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.entry()?.set_password(password).map_err(from_keyring)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = password;
            Err(unsupported())
        }
    }

    /// Read the password back.
    pub fn password(&self) -> Result<String, CredentialError> {
        self.check()?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.entry()?.get_password().map_err(from_keyring)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(unsupported())
        }
    }

    /// Store arbitrary bytes — a key, a refresh token, anything that is not
    /// text.
    pub fn set_secret(&self, secret: &[u8]) -> Result<(), CredentialError> {
        self.check()?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.entry()?.set_secret(secret).map_err(from_keyring)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = secret;
            Err(unsupported())
        }
    }

    /// Read the bytes back.
    pub fn secret(&self) -> Result<Vec<u8>, CredentialError> {
        self.check()?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.entry()?.get_secret().map_err(from_keyring)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(unsupported())
        }
    }

    /// Remove the entry.
    ///
    /// Deleting something that was never there is [`CredentialError::NotFound`]
    /// rather than success, so "sign out" can tell the difference between
    /// "removed" and "there was nothing to remove".
    pub fn delete(&self) -> Result<(), CredentialError> {
        self.check()?;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.entry()?.delete_credential().map_err(from_keyring)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(unsupported())
        }
    }

    /// Whether something is stored here.
    ///
    /// A read that reports [`CredentialError::NotFound`] answers `false`;
    /// anything else is still an error, because "the keychain is locked" is not
    /// the same as "there is nothing there".
    pub fn exists(&self) -> Result<bool, CredentialError> {
        match self.secret() {
            Ok(_) => Ok(true),
            Err(CredentialError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn entry(&self) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(&self.service, &self.account).map_err(from_keyring)
    }
}

/// The error for a platform with no store, with the reason in it.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn unsupported() -> CredentialError {
    CredentialError::Unsupported(
        "the Secret Service backend needs system libdbus at build time, and the kernel keyring \
         does not survive a reboot; neither is enabled behind the application's back"
            .into(),
    )
}

/// Translate a `keyring` error into ours.
///
/// Split out as a free function so the mapping is unit-testable without a
/// keychain: this is the part that decides whether "nothing saved yet" reads as
/// a first run or as a crash.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn from_keyring(e: keyring::Error) -> CredentialError {
    match e {
        keyring::Error::NoEntry => CredentialError::NotFound,
        keyring::Error::NoStorageAccess(_) => CredentialError::Locked,
        keyring::Error::BadEncoding(_) => CredentialError::NotText,
        other => CredentialError::Os(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Biometrics
// ---------------------------------------------------------------------------

/// Which biometric the machine offers.
///
/// ```
/// use silka_platform::credential::BiometricKind;
///
/// // "None" is a real answer, not a failure: a desktop with no sensor is a
/// // perfectly ordinary machine.
/// assert_eq!(BiometricKind::default(), BiometricKind::None);
/// assert!(!BiometricKind::None.is_available());
/// assert!(BiometricKind::TouchId.is_available());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BiometricKind {
    /// No sensor, or none the application may use.
    #[default]
    None,
    /// Touch ID.
    TouchId,
    /// Face ID.
    FaceId,
    /// Windows Hello (fingerprint, face, or PIN — the OS decides).
    WindowsHello,
}

impl BiometricKind {
    /// Whether there is anything to prompt with.
    pub const fn is_available(self) -> bool {
        !matches!(self, BiometricKind::None)
    }
}

/// Why an authentication attempt did not succeed.
///
/// [`BiometricError::Cancelled`] and [`BiometricError::Failed`] are different on
/// purpose: a user who pressed Escape has not failed anything, and an
/// application that treats the two the same ends up locking people out for
/// changing their mind.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BiometricError {
    /// The user dismissed the prompt.
    Cancelled,
    /// The biometric did not match.
    Failed,
    /// There is no sensor, or the application may not use it.
    Unavailable,
    /// No backend on this build. The message says what each platform needs.
    Unsupported(String),
    /// Anything else the OS reported.
    Os(String),
}

impl fmt::Display for BiometricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BiometricError::Cancelled => write!(f, "the user dismissed the prompt"),
            BiometricError::Failed => write!(f, "the biometric did not match"),
            BiometricError::Unavailable => write!(f, "no biometric sensor available"),
            BiometricError::Unsupported(m) => write!(f, "no biometric backend: {m}"),
            BiometricError::Os(m) => write!(f, "the biometric prompt failed: {m}"),
        }
    }
}

impl std::error::Error for BiometricError {}

/// What the machine offers, as far as this build can tell.
///
/// Always [`BiometricKind::None`] today — the backends named in
/// [`BiometricPrompt`] are not written. It is a function rather than a constant
/// because the answer depends on hardware, and a caller written against it now
/// keeps working when the backends land.
pub fn biometric_kind() -> BiometricKind {
    BiometricKind::None
}

/// A request for the user to prove they are present.
///
/// The reason is **not** decoration: macOS shows it verbatim in the system
/// prompt ("Editor is trying to unlock your saved sign-in"), and a prompt with
/// a vague reason is one users learn to dismiss.
///
/// ```
/// use silka_platform::credential::{biometric_prompt, BiometricError};
///
/// let prompt = biometric_prompt("unlock your saved sign-in")
///     .fallback("Use password…");
/// assert_eq!(prompt.reason(), "unlock your saved sign-in");
///
/// // Honest about not being wired up yet, rather than silently succeeding —
/// // which for an authorisation gesture would be the worst possible bug.
/// assert!(matches!(prompt.authenticate(), Err(BiometricError::Unsupported(_))));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiometricPrompt {
    reason: String,
    fallback: Option<String>,
}

/// Ask the user to prove they are present.
pub fn biometric_prompt(reason: impl Into<String>) -> BiometricPrompt {
    BiometricPrompt {
        reason: reason.into(),
        fallback: None,
    }
}

impl BiometricPrompt {
    /// The label of the "use my password instead" button.
    pub fn fallback(mut self, title: impl Into<String>) -> Self {
        self.fallback = Some(title.into());
        self
    }

    /// The reason shown to the user.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The fallback button's label, when one was set.
    pub fn fallback_title(&self) -> Option<&str> {
        self.fallback.as_deref()
    }

    /// Show the prompt and wait for an answer.
    ///
    /// # Errors
    ///
    /// Always [`BiometricError::Unsupported`] today. macOS needs
    /// `LAContext::evaluatePolicy:localizedReason:reply:` from
    /// LocalAuthentication, and Windows needs the WinRT
    /// `UserConsentVerifier` — neither of which is in the binding set this
    /// workspace pins. **Failing closed is the only safe seam** for an
    /// authorisation gesture: an `Ok(())` placeholder would be a security hole
    /// that looks like progress.
    pub fn authenticate(&self) -> Result<(), BiometricError> {
        Err(BiometricError::Unsupported(
            "macOS needs LocalAuthentication (LAContext) and Windows needs the WinRT \
             UserConsentVerifier; neither binding is pinned by this workspace"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nama_kosong_ditolak_sebelum_menyentuh_keychain() {
        // An empty half would make every caller share one slot.
        assert_eq!(
            credential("", "ana").check(),
            Err(CredentialError::EmptyName)
        );
        assert_eq!(
            credential("app", " ").check(),
            Err(CredentialError::EmptyName)
        );
        assert!(credential("app", "ana").check().is_ok());
    }

    #[test]
    fn kredensial_menyimpan_kedua_bagian_namanya() {
        let c = credential("com.example.editor", "ana@example.com");
        assert_eq!(c.service(), "com.example.editor");
        assert_eq!(c.account(), "ana@example.com");
    }

    #[test]
    fn dua_akun_di_layanan_yang_sama_bukan_slot_yang_sama() {
        // The bug this pair of names exists to prevent.
        assert_ne!(credential("app", "ana"), credential("app", "budi"));
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn belum_ada_yang_tersimpan_bukan_kegagalan_fatal() {
        assert_eq!(
            from_keyring(keyring::Error::NoEntry),
            CredentialError::NotFound
        );
        assert_eq!(
            from_keyring(keyring::Error::BadEncoding(vec![0xFF])),
            CredentialError::NotText
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn linux_menolak_dengan_alasan_bukan_dengan_diam() {
        // The whole point: an application must not think it saved a token to
        // the login keyring when it saved it to a HashMap.
        let e = credential("app", "ana").password();
        assert!(matches!(e, Err(CredentialError::Unsupported(_))));
        assert!(!is_supported());
    }

    #[test]
    fn dukungan_bisa_ditanya_lebih_dulu() {
        assert_eq!(
            is_supported(),
            cfg!(any(target_os = "macos", target_os = "windows"))
        );
    }

    #[test]
    fn biometrik_gagal_tertutup_bukan_terbuka() {
        // An `Ok(())` placeholder for an authorisation gesture would be a
        // security hole that looks like progress.
        let prompt = biometric_prompt("unlock your saved sign-in");
        assert!(matches!(
            prompt.authenticate(),
            Err(BiometricError::Unsupported(_))
        ));
        assert!(!biometric_kind().is_available());
    }

    #[test]
    fn prompt_membawa_alasan_dan_tombol_cadangannya() {
        let p = biometric_prompt("buka sesi tersimpan").fallback("Pakai kata sandi…");
        assert_eq!(p.reason(), "buka sesi tersimpan");
        assert_eq!(p.fallback_title(), Some("Pakai kata sandi…"));
        assert_eq!(biometric_prompt("x").fallback_title(), None);
    }

    #[test]
    fn batal_dan_gagal_bukan_hal_yang_sama() {
        // Treating the two alike locks people out for changing their mind.
        assert_ne!(BiometricError::Cancelled, BiometricError::Failed);
    }
}
