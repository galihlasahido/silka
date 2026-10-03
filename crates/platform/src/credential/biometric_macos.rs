//! Touch ID / Face ID / Optic ID through LocalAuthentication's `LAContext`.
//!
//! `objc2-local-authentication` types stay inside this file, the same way
//! `keyring` stays inside the parent module.
//!
//! ## It blocks
//!
//! `evaluatePolicy:localizedReason:reply:` is asynchronous: the system shows
//! its prompt out of process and calls a reply block on a private queue. The
//! public API above is synchronous, so [`authenticate`] parks the calling
//! thread on a channel until that block runs. Calling it from the main thread
//! is fine — the reply never needs the main thread — but the application's
//! own UI does not repaint while it waits. Run it from a worker thread
//! (`silka_core::task`) when that matters.
//!
//! ## Every path except one fails closed
//!
//! Only the reply block saying `success == YES` yields `Ok(())`. A reply that
//! never arrives is impossible to distinguish from a hang, so a dropped sender
//! (the block released without running) is reported as an error, never as
//! success.

use std::sync::mpsc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LABiometryType, LAContext, LAError, LAPolicy};

use super::{BiometricError, BiometricKind};

/// Biometrics only: the OS "enter your password" fallback would let a typed
/// password satisfy a gesture that is documented as proving presence of the
/// enrolled person.
const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

/// What the machine offers right now.
pub(super) fn kind() -> BiometricKind {
    // SAFETY: `new` and `canEvaluatePolicy:error:` have no preconditions;
    // `biometryType` is only meaningful after a successful `canEvaluate`,
    // which is exactly the order used here.
    unsafe {
        let context = LAContext::new();
        if context.canEvaluatePolicy_error(POLICY).is_err() {
            return BiometricKind::None;
        }
        match context.biometryType() {
            LABiometryType::TouchID => BiometricKind::TouchId,
            LABiometryType::FaceID => BiometricKind::FaceId,
            LABiometryType::OpticID => BiometricKind::OpticId,
            _ => BiometricKind::None,
        }
    }
}

/// Show the system prompt and wait for the answer.
pub(super) fn authenticate(reason: &str, fallback: Option<&str>) -> Result<(), BiometricError> {
    let (tx, rx) = mpsc::channel::<Result<(), BiometricError>>();

    // SAFETY: `LAContext` is created and used on this thread only and kept
    // alive (strong reference) until the reply arrives — dropping it earlier
    // would cancel the evaluation. The reply block only captures a `Sender`,
    // which is `Send`, as the method requires of a block that runs on another
    // queue.
    unsafe {
        let context: Retained<LAContext> = LAContext::new();

        // An empty title hides the fallback button; leaving it unset shows the
        // system default, which is not what "no fallback" means.
        let title = NSString::from_str(fallback.unwrap_or(""));
        context.setLocalizedFallbackTitle(Some(&title));

        if let Err(e) = context.canEvaluatePolicy_error(POLICY) {
            return Err(map_error(e.code()));
        }

        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(())
            } else {
                // On failure the system passes a valid `NSError`, or null;
                // null is handled as a generic failure.
                match error.as_ref() {
                    Some(e) => Err(map_error(e.code())),
                    None => Err(BiometricError::Failed),
                }
            };
            // The receiver may have gone away; nothing to do about it.
            let _ = tx.send(outcome);
        });

        let reason = NSString::from_str(reason);
        context.evaluatePolicy_localizedReason_reply(POLICY, &reason, &reply);

        rx.recv().unwrap_or_else(|_| {
            Err(BiometricError::Os(
                "LocalAuthentication released its reply without answering".into(),
            ))
        })
    }
}

/// Translate an `LAError` code. Split out so the mapping is unit-testable
/// without a sensor.
pub(super) fn map_error(code: isize) -> BiometricError {
    let code = LAError(code);
    match code {
        LAError::UserCancel
        | LAError::SystemCancel
        | LAError::AppCancel
        | LAError::UserFallback => BiometricError::Cancelled,
        LAError::AuthenticationFailed | LAError::BiometryLockout => BiometricError::Failed,
        LAError::BiometryNotAvailable
        | LAError::BiometryNotEnrolled
        | LAError::PasscodeNotSet
        | LAError::NotInteractive => BiometricError::Unavailable,
        other => BiometricError::Os(format!("LocalAuthentication error {}", other.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_is_not_failing() {
        assert_eq!(map_error(-2), BiometricError::Cancelled);
        assert_eq!(map_error(-4), BiometricError::Cancelled);
        assert_eq!(map_error(-9), BiometricError::Cancelled);
        assert_eq!(map_error(-3), BiometricError::Cancelled);
    }

    #[test]
    fn a_mismatch_and_a_lockout_are_failures() {
        assert_eq!(map_error(-1), BiometricError::Failed);
        assert_eq!(map_error(-8), BiometricError::Failed);
    }

    #[test]
    fn no_sensor_or_enrolment_is_unavailable() {
        for code in [-5, -6, -7, -1004] {
            assert_eq!(map_error(code), BiometricError::Unavailable, "{code}");
        }
    }

    #[test]
    fn an_unknown_code_is_never_success() {
        assert!(matches!(map_error(-9999), BiometricError::Os(_)));
    }
}
