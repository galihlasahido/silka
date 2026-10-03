//! Windows Hello through the WinRT `UserConsentVerifier`.
//!
//! `windows::` types stay inside this file.
//!
//! ## It blocks
//!
//! Both WinRT calls are `IAsyncOperation`s; [`authenticate`] joins them, so
//! the calling thread waits for the user. Run it from a worker thread when the
//! UI must keep painting. `join` must not be used on a single-threaded
//! apartment that is also expected to pump the prompt's messages — worker
//! threads are multithreaded-apartment by default, which is the supported
//! shape.
//!
//! ## Honest limitation
//!
//! This uses `RequestVerificationAsync`, which takes no window handle. For an
//! unpackaged desktop application Windows may show the Hello dialog behind
//! the application window; the window-parented variant
//! (`IUserConsentVerifierInterop::RequestVerificationForWindowAsync`) needs an
//! `HWND`, which this credential API does not receive. Every outcome still
//! fails closed: only `Verified` yields `Ok(())`.

use windows::core::HSTRING;
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};

use super::{BiometricError, BiometricKind};

/// What the machine offers right now.
pub(super) fn kind() -> BiometricKind {
    match availability() {
        Ok(UserConsentVerifierAvailability::Available) => BiometricKind::WindowsHello,
        _ => BiometricKind::None,
    }
}

fn availability() -> windows::core::Result<UserConsentVerifierAvailability> {
    UserConsentVerifier::CheckAvailabilityAsync()?.join()
}

/// Show Windows Hello and wait for the answer.
///
/// Windows has no fallback-button text to set; `fallback` is accepted for a
/// uniform signature and ignored (Hello offers its own PIN path).
pub(super) fn authenticate(reason: &str, _fallback: Option<&str>) -> Result<(), BiometricError> {
    match availability() {
        Ok(UserConsentVerifierAvailability::Available) => {}
        Ok(_) => return Err(BiometricError::Unavailable),
        Err(e) => return Err(BiometricError::Os(e.message())),
    }
    let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(reason))
        .and_then(|op| op.join())
        .map_err(|e| BiometricError::Os(e.message()))?;
    map_result(result.0)
}

/// Translate a `UserConsentVerificationResult` value. Split out so the
/// mapping is unit-testable without Hello.
pub(super) fn map_result(value: i32) -> Result<(), BiometricError> {
    match UserConsentVerificationResult(value) {
        UserConsentVerificationResult::Verified => Ok(()),
        UserConsentVerificationResult::Canceled => Err(BiometricError::Cancelled),
        UserConsentVerificationResult::RetriesExhausted => Err(BiometricError::Failed),
        UserConsentVerificationResult::DeviceNotPresent
        | UserConsentVerificationResult::NotConfiguredForUser
        | UserConsentVerificationResult::DisabledByPolicy => Err(BiometricError::Unavailable),
        UserConsentVerificationResult::DeviceBusy => {
            Err(BiometricError::Os("the biometric device is busy".into()))
        }
        other => Err(BiometricError::Os(format!(
            "unknown verification result {}",
            other.0
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_verified_is_success() {
        assert_eq!(map_result(0), Ok(()));
        for v in 1..=8 {
            assert!(map_result(v).is_err(), "{v}");
        }
    }

    #[test]
    fn cancelling_is_not_failing() {
        assert_eq!(map_result(6), Err(BiometricError::Cancelled));
        assert_eq!(map_result(5), Err(BiometricError::Failed));
    }
}
