#!/usr/bin/env bash
# Build the update feed (SISA-PEKERJAAN §I4).
#
# This is the writer for the document `silka_dist::feed::Feed::parse` reads, and
# the two are meant to be read side by side: every field this script emits is
# defended in a comment in crates/dist/src/feed.rs.
#
# Usage:
#
#   .github/scripts/make-update-feed.sh \
#       --app dev.silka.dashboard \
#       --version 1.4.0 \
#       --channel stable \
#       --base-url https://downloads.example.com/v1.4.0 \
#       --dir dist \
#       --out feed.json \
#       [--previous old-feed.json] \
#       [--rollout 25] [--mandatory] [--notes https://…/1.4.0.html] \
#       [--min-macos 11.0] [--min-windows 10.0.19041] [--min-linux ""] \
#       [--sign-key private.pem]
#
# ---------------------------------------------------------------------------
# The three things it gets right that a hand-written feed gets wrong
# ---------------------------------------------------------------------------
#
#   1. **It merges.** A feed lists every release, not the newest one. Passing
#      `--previous` carries the old entries forward and replaces any entry with
#      the same version, so re-running a failed release job is idempotent rather
#      than duplicating a release.
#
#   2. **It signs the digest, not the file.** The signature covers the 32 raw
#      bytes of the SHA-256, which is exactly what
#      `silka_dist::update::SignatureVerifier::verify` is handed. Signing the
#      file instead would mean the verifier had to hold a 200 MB artifact in
#      memory to check it.
#
#   3. **It refuses to guess.** A file in `--dir` whose platform cannot be
#      determined from its name is an error, not a skipped artifact. A feed that
#      quietly omits the Windows build ships an update nobody on Windows gets,
#      and nothing about that failure is visible until the support tickets.
#
# ---------------------------------------------------------------------------
# The signing key
# ---------------------------------------------------------------------------
#
# Ed25519, generated once and kept out of every repository:
#
#   openssl genpkey -algorithm ed25519 -out update-signing.pem
#   openssl pkey -in update-signing.pem -pubout -out update-signing.pub
#
# The private half goes into ${{ secrets.UPDATE_SIGNING_KEY }}; the public half
# is compiled into the application, which is what makes the check meaningful —
# a public key fetched at runtime is a public key an attacker can replace.
#
# Rotating it is a two-release job: ship a build that trusts both keys, wait for
# it to reach everyone, then sign with the new one only.

set -euo pipefail

APP=""
VERSION=""
CHANNEL="stable"
BASE_URL=""
DIR="dist"
OUT="feed.json"
PREVIOUS=""
ROLLOUT="100"
MANDATORY="false"
NOTES=""
MIN_MACOS=""
MIN_WINDOWS=""
MIN_LINUX=""
SIGN_KEY=""
PUBLISHED="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --app)          APP="$2"; shift 2 ;;
    --version)      VERSION="$2"; shift 2 ;;
    --channel)      CHANNEL="$2"; shift 2 ;;
    --base-url)     BASE_URL="${2%/}"; shift 2 ;;
    --dir)          DIR="$2"; shift 2 ;;
    --out)          OUT="$2"; shift 2 ;;
    --previous)     PREVIOUS="$2"; shift 2 ;;
    --rollout)      ROLLOUT="$2"; shift 2 ;;
    --mandatory)    MANDATORY="true"; shift ;;
    --notes)        NOTES="$2"; shift 2 ;;
    --min-macos)    MIN_MACOS="$2"; shift 2 ;;
    --min-windows)  MIN_WINDOWS="$2"; shift 2 ;;
    --min-linux)    MIN_LINUX="$2"; shift 2 ;;
    --sign-key)     SIGN_KEY="$2"; shift 2 ;;
    --published)    PUBLISHED="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

: "${APP:?--app is required}"
: "${VERSION:?--version is required}"
: "${BASE_URL:?--base-url is required}"

if [ ! -d "$DIR" ]; then
  echo "artifact directory not found: $DIR" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ---------------------------------------------------------------------------
# One line per artifact: platform<TAB>format<TAB>name<TAB>size<TAB>sha256<TAB>signature
# ---------------------------------------------------------------------------

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    sha256sum "$1" | cut -d' ' -f1
  fi
}

sign_digest() {
  # $1 = hex digest. Signs the raw 32 bytes it spells, base64 of the signature
  # on stdout. Empty when no key was supplied — the feed then carries no
  # `signature` field, and `Download::finish` refuses such an artifact the
  # moment the application has a verifier.
  [ -z "$SIGN_KEY" ] && return 0

  local raw="$WORK/digest.bin"
  local sig="$WORK/digest.sig"
  # `xxd -r -p` turns the hex back into the bytes the verifier will be handed.
  printf '%s' "$1" | xxd -r -p > "$raw"
  openssl pkeyutl -sign -inkey "$SIGN_KEY" -rawin -in "$raw" -out "$sig"
  base64 < "$sig" | tr -d '\n'
}

ARTIFACTS="$WORK/artifacts.tsv"
: > "$ARTIFACTS"

for FILE in "$DIR"/*; do
  [ -f "$FILE" ] || continue
  NAME="$(basename "$FILE")"

  case "$NAME" in
    *.json|*.txt|*.sha256|*.sig) continue ;;   # sidecars, not artifacts
  esac

  # The mapping from a file name to a feed platform. Deliberately explicit:
  # these strings have to match `silka_dist::feed::Platform::parse`, and a typo
  # here produces a feed that parses and serves nobody.
  case "$NAME" in
    *aarch64*.dmg|*arm64*.dmg)          PLATFORM="macos-aarch64";  FORMAT="dmg" ;;
    *x86_64*.dmg|*x64*.dmg|*intel*.dmg) PLATFORM="macos-x86_64";   FORMAT="dmg" ;;
    *.dmg)                              PLATFORM="macos-universal"; FORMAT="dmg" ;;
    *.pkg)                              PLATFORM="macos-universal"; FORMAT="pkg" ;;
    *arm64*.msi|*aarch64*.msi)          PLATFORM="windows-aarch64"; FORMAT="msi" ;;
    *.msi)                              PLATFORM="windows-x86_64";  FORMAT="msi" ;;
    *arm64*.exe|*aarch64*.exe)          PLATFORM="windows-aarch64"; FORMAT="exe" ;;
    *.exe)                              PLATFORM="windows-x86_64";  FORMAT="exe" ;;
    *aarch64*.AppImage|*arm64*.AppImage) PLATFORM="linux-aarch64"; FORMAT="AppImage" ;;
    *.AppImage)                         PLATFORM="linux-x86_64";   FORMAT="AppImage" ;;
    *arm64*.deb|*aarch64*.deb)          PLATFORM="linux-aarch64";  FORMAT="deb" ;;
    *.deb)                              PLATFORM="linux-x86_64";   FORMAT="deb" ;;
    *aarch64*.rpm)                      PLATFORM="linux-aarch64";  FORMAT="rpm" ;;
    *.rpm)                              PLATFORM="linux-x86_64";   FORMAT="rpm" ;;
    *)
      echo "cannot tell what platform $NAME is for." >&2
      echo "add a rule to make-update-feed.sh rather than letting it be skipped." >&2
      exit 1
      ;;
  esac

  SIZE="$(wc -c < "$FILE" | tr -d ' ')"
  DIGEST="$(sha256_of "$FILE")"
  SIGNATURE="$(sign_digest "$DIGEST")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$PLATFORM" "$FORMAT" "$NAME" "$SIZE" "$DIGEST" "$SIGNATURE" >> "$ARTIFACTS"
  echo "==> $NAME  $PLATFORM/$FORMAT  $SIZE bytes  ${DIGEST:0:16}…"
done

if [ ! -s "$ARTIFACTS" ]; then
  echo "no artifacts found in $DIR" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Assembly
#
# Python rather than more shell: merging two JSON documents with `sed` is how a
# feed ends up with a duplicated release nobody notices for a week. Python 3 is
# present on all three GitHub runner images.
# ---------------------------------------------------------------------------

APP="$APP" VERSION="$VERSION" CHANNEL="$CHANNEL" BASE_URL="$BASE_URL" \
PUBLISHED="$PUBLISHED" ROLLOUT="$ROLLOUT" MANDATORY="$MANDATORY" NOTES="$NOTES" \
MIN_MACOS="$MIN_MACOS" MIN_WINDOWS="$MIN_WINDOWS" MIN_LINUX="$MIN_LINUX" \
ARTIFACTS="$ARTIFACTS" PREVIOUS="$PREVIOUS" OUT="$OUT" \
python3 - <<'PYTHON'
import json
import os
import sys

app = os.environ["APP"]
version = os.environ["VERSION"]
channel = os.environ["CHANNEL"]
base_url = os.environ["BASE_URL"]

artifacts = []
with open(os.environ["ARTIFACTS"], encoding="utf-8") as handle:
    for line in handle:
        line = line.rstrip("\n")
        if not line:
            continue
        platform, fmt, name, size, digest, signature = line.split("\t")
        entry = {
            "platform": platform,
            "format": fmt,
            "url": f"{base_url}/{name}",
            "size": int(size),
            "sha256": digest,
        }
        # An empty signature is omitted rather than written as "": the reader
        # treats a missing field and an empty one differently, and only one of
        # those is honest about there being no signature.
        if signature:
            entry["signature"] = signature
        artifacts.append(entry)

# Artifacts sorted by platform so two runs over the same inputs produce the same
# bytes. A feed that differs only in ordering makes every diff unreadable.
artifacts.sort(key=lambda entry: (entry["platform"], entry["format"]))

release = {
    "version": version,
    "published": os.environ["PUBLISHED"],
    "mandatory": os.environ["MANDATORY"] == "true",
    "rollout": int(os.environ["ROLLOUT"]),
    "artifacts": artifacts,
}
if os.environ.get("NOTES"):
    release["notes"] = os.environ["NOTES"]

minimum = {}
for key, name in (("MIN_MACOS", "macos"), ("MIN_WINDOWS", "windows"), ("MIN_LINUX", "linux")):
    value = os.environ.get(key, "").strip()
    if value:
        minimum[name] = value
if minimum:
    release["minimum_os"] = minimum

releases = []
previous = os.environ.get("PREVIOUS", "")
if previous:
    with open(previous, encoding="utf-8") as handle:
        old = json.load(handle)
    if old.get("app") != app:
        sys.exit(f"previous feed is for {old.get('app')!r}, not {app!r}")
    if old.get("channel") != channel:
        sys.exit(f"previous feed serves {old.get('channel')!r}, not {channel!r}")
    # Re-running a release replaces its entry instead of adding a second one.
    releases = [entry for entry in old.get("releases", []) if entry.get("version") != version]

releases.append(release)

feed = {
    "feed": 1,
    "app": app,
    "channel": channel,
    "releases": releases,
}

with open(os.environ["OUT"], "w", encoding="utf-8") as handle:
    json.dump(feed, handle, indent=2, ensure_ascii=False, sort_keys=False)
    handle.write("\n")
PYTHON

# The last gate before upload: read the feed back the way the application will.
# `silka-dist` has no dependencies, so this compiles in seconds and it is the
# only check that exercises the same parser the shipped binary uses.
if command -v cargo >/dev/null 2>&1; then
  echo "==> reading $OUT back through silka_dist::feed::Feed::parse"
  REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  FEED_PATH="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")" \
    cargo test -q --manifest-path "$REPO_ROOT/Cargo.toml" -p silka-dist \
      --test feed_terbaca -- --nocapture
fi

echo "==> wrote $OUT"
