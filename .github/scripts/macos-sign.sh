#!/usr/bin/env bash
# Code signing for macOS (SISA-PEKERJAAN §I1).
#
# Usage:  .github/scripts/macos-sign.sh <path to .app or binary> [more paths...]
#
# Without this, two things break — and the second one surprises people:
#
#   1. Gatekeeper refuses to launch the application at all on any Mac that did
#      not build it.
#   2. **System notifications silently do nothing.** UNUserNotificationCenter
#      requires a signed bundle with a stable identifier; on an unsigned build
#      the request is accepted and nothing appears, which is exactly what
#      `silka_platform::notification::needs_bundle()` exists to warn about.
#
# ---------------------------------------------------------------------------
# Secrets this script reads (never write any of these into a file in the repo)
# ---------------------------------------------------------------------------
#
#   MACOS_CERT_P12          base64 of the Developer ID Application .p12
#   MACOS_CERT_PASSWORD     password that .p12 was exported with
#   MACOS_KEYCHAIN_PASSWORD any string; the temporary keychain's own password
#   MACOS_SIGN_IDENTITY     e.g. "Developer ID Application: Acme Ltd (AB12CD34EF)"
#
# In the workflow they arrive as ${{ secrets.MACOS_CERT_P12 }} and friends.
# Produce the first one with:
#
#   base64 -i DeveloperID.p12 | pbcopy
#
# ---------------------------------------------------------------------------
# Three decisions worth knowing before editing
# ---------------------------------------------------------------------------
#
#   * A **temporary keychain**, deleted by the trap below. Importing into the
#     login keychain leaves a certificate on a shared runner, and on a
#     self-hosted runner it leaves it forever.
#
#   * `--options runtime` (the hardened runtime) is not optional: notarization
#     refuses a bundle without it. Its entitlements are in
#     packaging/macos/entitlements.plist, one comment per hole.
#
#   * **No `--deep`.** Apple documents it as unsuitable for signing shipping
#     software: it re-signs nested code with the *outer* bundle's entitlements,
#     which silently widens every helper's permissions. This script walks the
#     bundle and signs from the inside out instead.

set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <path to .app or binary> [more paths...]" >&2
  exit 2
fi

: "${MACOS_CERT_P12:?set MACOS_CERT_P12 (base64 of the .p12)}"
: "${MACOS_CERT_PASSWORD:?set MACOS_CERT_PASSWORD}"
: "${MACOS_KEYCHAIN_PASSWORD:?set MACOS_KEYCHAIN_PASSWORD}"
: "${MACOS_SIGN_IDENTITY:?set MACOS_SIGN_IDENTITY}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENTITLEMENTS="${MACOS_ENTITLEMENTS:-$REPO_ROOT/packaging/macos/entitlements.plist}"

if [ ! -f "$ENTITLEMENTS" ]; then
  echo "entitlements not found: $ENTITLEMENTS" >&2
  exit 1
fi

KEYCHAIN="$(mktemp -d)/silka-signing.keychain-db"
CERTIFICATE="$(mktemp -d)/certificate.p12"

cleanup() {
  # Runs on success and on failure. A runner that dies between `import` and
  # `delete-keychain` still leaves the keychain behind, which is why the
  # keychain is temporary rather than the login one.
  security delete-keychain "$KEYCHAIN" 2>/dev/null || true
  rm -f "$CERTIFICATE"
}
trap cleanup EXIT

echo "==> creating a temporary keychain"
security create-keychain -p "$MACOS_KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 3600 "$KEYCHAIN"
security unlock-keychain -p "$MACOS_KEYCHAIN_PASSWORD" "$KEYCHAIN"

# Prepend rather than replace: `security default-keychain -s` would hide the
# runner's own keychain from every other tool for the rest of the job.
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')

echo "==> importing the signing certificate"
printf '%s' "$MACOS_CERT_P12" | base64 --decode > "$CERTIFICATE"
security import "$CERTIFICATE" \
  -k "$KEYCHAIN" \
  -P "$MACOS_CERT_PASSWORD" \
  -T /usr/bin/codesign \
  -T /usr/bin/security

# Without this, codesign blocks on a GUI prompt that no CI runner can answer,
# and the job hangs until it times out rather than failing.
security set-key-partition-list \
  -S apple-tool:,apple:,codesign: \
  -s -k "$MACOS_KEYCHAIN_PASSWORD" \
  "$KEYCHAIN" >/dev/null

echo "==> identity available:"
security find-identity -v -p codesigning "$KEYCHAIN"

sign_one() {
  codesign \
    --force \
    --sign "$MACOS_SIGN_IDENTITY" \
    --keychain "$KEYCHAIN" \
    --options runtime \
    --timestamp \
    --entitlements "$ENTITLEMENTS" \
    --generate-entitlement-der \
    "$1"
}

for TARGET in "$@"; do
  if [ ! -e "$TARGET" ]; then
    echo "nothing to sign at $TARGET" >&2
    exit 1
  fi

  echo "==> signing $TARGET"
  if [ -d "$TARGET" ]; then
    # Inside out: nested code must already be signed when the outer bundle is
    # sealed, or its seal covers a signature that is about to change.
    while IFS= read -r -d '' NESTED; do
      echo "    nested: $NESTED"
      sign_one "$NESTED"
    done < <(
      find "$TARGET/Contents" \
        \( -name '*.dylib' -o -name '*.so' -o -name '*.framework' \) \
        -print0 2>/dev/null || true
    )
    if [ -d "$TARGET/Contents/MacOS" ]; then
      while IFS= read -r -d '' HELPER; do
        echo "    executable: $HELPER"
        sign_one "$HELPER"
      done < <(find "$TARGET/Contents/MacOS" -type f -perm -u+x -print0)
    fi
  fi
  sign_one "$TARGET"

  echo "==> verifying $TARGET"
  # `--strict` is the difference between "there is a signature" and "the
  # signature covers everything in the bundle".
  codesign --verify --deep --strict --verbose=2 "$TARGET"
  codesign --display --entitlements :- "$TARGET" >/dev/null
done

echo "==> signed $# path(s)"
echo "    Gatekeeper will still refuse these until they are notarized:"
echo "    .github/scripts/macos-notarize.sh"
