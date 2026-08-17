#!/usr/bin/env bash
# Notarization and stapling for macOS (SISA-PEKERJAAN §I1).
#
# Usage:  .github/scripts/macos-notarize.sh <path to .dmg, .pkg or .zip> [more...]
#
# Signing proves *who* built it. Notarization is Apple scanning the build and
# issuing a ticket that says so; stapling attaches that ticket to the file, which
# is what lets a Mac verify it **offline**. Skip the staple and the first launch
# on a machine with no network shows the "cannot be opened" dialog even though
# the build is perfectly notarized.
#
# Submit an archive, not a bare `.app`: notarytool takes `.dmg`, `.pkg` and
# `.zip` only. And staple the same file you shipped — a ticket stapled to a
# `.app` inside a `.dmg` you then rebuild is a ticket on a file nobody has.
#
# ---------------------------------------------------------------------------
# Credentials
# ---------------------------------------------------------------------------
#
# Preferred — an App Store Connect API key. It is scoped, it is revocable, and
# it does not carry a human's Apple ID:
#
#   NOTARY_API_KEY_P8    base64 of the AuthKey_XXXXXXXX.p8 file
#   NOTARY_API_KEY_ID    the key id, the XXXXXXXX in that file name
#   NOTARY_API_ISSUER    the issuer UUID from App Store Connect
#
# Fallback — an Apple ID with an app-specific password. Works, but it is a
# person's account and it stops working the day they leave:
#
#   NOTARY_APPLE_ID      the Apple ID e-mail
#   NOTARY_PASSWORD      an app-specific password (NOT the account password)
#   NOTARY_TEAM_ID       the ten-character team id
#
# In the workflow: ${{ secrets.NOTARY_API_KEY_P8 }} and friends.

set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <path to .dmg, .pkg or .zip> [more...]" >&2
  exit 2
fi

KEY_FILE=""
cleanup() {
  [ -n "$KEY_FILE" ] && rm -f "$KEY_FILE"
  return 0
}
trap cleanup EXIT

NOTARY_ARGS=()
if [ -n "${NOTARY_API_KEY_P8:-}" ]; then
  : "${NOTARY_API_KEY_ID:?set NOTARY_API_KEY_ID alongside NOTARY_API_KEY_P8}"
  : "${NOTARY_API_ISSUER:?set NOTARY_API_ISSUER alongside NOTARY_API_KEY_P8}"

  KEY_FILE="$(mktemp -d)/AuthKey.p8"
  printf '%s' "$NOTARY_API_KEY_P8" | base64 --decode > "$KEY_FILE"
  chmod 600 "$KEY_FILE"
  NOTARY_ARGS=(--key "$KEY_FILE" --key-id "$NOTARY_API_KEY_ID" --issuer "$NOTARY_API_ISSUER")
  echo "==> authenticating with an App Store Connect API key"
elif [ -n "${NOTARY_APPLE_ID:-}" ]; then
  : "${NOTARY_PASSWORD:?set NOTARY_PASSWORD alongside NOTARY_APPLE_ID}"
  : "${NOTARY_TEAM_ID:?set NOTARY_TEAM_ID alongside NOTARY_APPLE_ID}"

  NOTARY_ARGS=(
    --apple-id "$NOTARY_APPLE_ID"
    --password "$NOTARY_PASSWORD"
    --team-id "$NOTARY_TEAM_ID"
  )
  echo "==> authenticating with an Apple ID and an app-specific password"
else
  echo "no notarization credentials: set NOTARY_API_KEY_P8 (+ID, +ISSUER) or NOTARY_APPLE_ID (+PASSWORD, +TEAM_ID)" >&2
  exit 1
fi

for TARGET in "$@"; do
  if [ ! -f "$TARGET" ]; then
    echo "nothing to notarize at $TARGET" >&2
    exit 1
  fi

  echo "==> submitting $TARGET"
  # `--wait` blocks until Apple answers. Minutes, usually; the alternative is a
  # release job that reports success before anyone knows whether it passed.
  SUBMISSION_JSON="$(mktemp)"
  set +e
  xcrun notarytool submit "$TARGET" \
    "${NOTARY_ARGS[@]}" \
    --wait \
    --timeout 45m \
    --output-format json > "$SUBMISSION_JSON"
  SUBMIT_STATUS=$?
  set -e

  SUBMISSION_ID="$(
    /usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("id",""))' \
      "$SUBMISSION_JSON" 2>/dev/null || true
  )"
  SUBMISSION_STATE="$(
    /usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("status",""))' \
      "$SUBMISSION_JSON" 2>/dev/null || true
  )"
  echo "    submission $SUBMISSION_ID: ${SUBMISSION_STATE:-unknown}"

  if [ "$SUBMIT_STATUS" -ne 0 ] || [ "$SUBMISSION_STATE" != "Accepted" ]; then
    # The log is the only thing that says *which* binary was unsigned or which
    # entitlement was rejected. Printing it here saves the next hour.
    echo "==> notarization did not pass; Apple's log follows"
    if [ -n "$SUBMISSION_ID" ]; then
      xcrun notarytool log "$SUBMISSION_ID" "${NOTARY_ARGS[@]}" || true
    fi
    exit 1
  fi

  echo "==> stapling the ticket to $TARGET"
  xcrun stapler staple "$TARGET"
  xcrun stapler validate "$TARGET"

  # The end-to-end check, and the only one that answers the question a user
  # asks: will this launch on a Mac that has never seen it?
  case "$TARGET" in
    *.dmg|*.zip)
      spctl --assess --type open --context context:primary-signature -vv "$TARGET" || true
      ;;
    *.pkg)
      spctl --assess --type install -vv "$TARGET" || true
      ;;
  esac
done

echo "==> notarized and stapled $# file(s)"
