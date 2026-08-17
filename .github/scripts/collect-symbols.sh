#!/usr/bin/env bash
# Collect debug symbols so a crash report can be read months later
# (SISA-PEKERJAAN §I5).
#
# Usage:
#
#   .github/scripts/collect-symbols.sh \
#       --binary target/release/silka-dashboard \
#       --platform macos-aarch64 \
#       --version 1.4.0 \
#       --build "$GITHUB_SHA" \
#       --out symbols
#
# ---------------------------------------------------------------------------
# Why this exists at all
# ---------------------------------------------------------------------------
#
# A release build's stack trace is a list of addresses. Turning it into function
# names needs the debug info that was thrown away when the binary shipped, and
# the *only* moment that debug info exists is on the machine that built it. A
# crash report collected six months from now is worth nothing unless this
# archive was kept — which is why this runs in the release job and not on
# demand.
#
# The layout matches `silka_dist::crash::CrashReport::symbol_path`:
#
#   symbols/<platform>/<version>/<build>/…
#
# so a report that says `macos-aarch64/1.4.0/9e75a29` names its own directory.
# That is the whole point of `CrashContext::build`: two builds of "1.4.0" have
# different symbols and the version alone cannot tell them apart.
#
# ---------------------------------------------------------------------------
# What `dump_syms` is
# ---------------------------------------------------------------------------
#
#   cargo install dump_syms
#
# It reads DWARF (Linux, macOS) or a PDB (Windows) and writes a Breakpad `.sym`
# file — the format `minidump-stackwalk`, Sentry and Socorro all consume. It is
# a build-time tool: nothing from it ships to a user.
#
# If it is not installed this script says so and stops rather than producing an
# empty archive. An empty symbol archive is worse than none: it looks like the
# step ran.

set -euo pipefail

BINARY=""
PLATFORM=""
VERSION=""
BUILD=""
OUT="symbols"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary)   BINARY="$2"; shift 2 ;;
    --platform) PLATFORM="$2"; shift 2 ;;
    --version)  VERSION="$2"; shift 2 ;;
    --build)    BUILD="$2"; shift 2 ;;
    --out)      OUT="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

: "${BINARY:?--binary is required}"
: "${PLATFORM:?--platform is required}"
: "${VERSION:?--version is required}"
: "${BUILD:?--build is required (a commit hash, or whatever CrashContext::build carries)}"

if [ ! -e "$BINARY" ]; then
  echo "no binary at $BINARY" >&2
  exit 1
fi

if ! command -v dump_syms >/dev/null 2>&1; then
  echo "dump_syms not found. Install it with:" >&2
  echo "    cargo install dump_syms" >&2
  echo "Refusing to write an empty symbol archive." >&2
  exit 1
fi

DESTINATION="$OUT/$PLATFORM/$VERSION/$BUILD"
mkdir -p "$DESTINATION"

NAME="$(basename "$BINARY")"

case "$PLATFORM" in
  macos-*)
    # The debug info lives in a `.dSYM` bundle beside the binary, and it only
    # exists if the build produced one. `dsymutil` builds it from the object
    # files cargo left in target/, which is why this has to run on the build
    # machine before anything is cleaned.
    DSYM="$BINARY.dSYM"
    if [ ! -d "$DSYM" ]; then
      echo "==> building $DSYM"
      dsymutil "$BINARY" -o "$DSYM"
    fi
    echo "==> dumping symbols from $DSYM"
    dump_syms "$DSYM" --output "$DESTINATION/$NAME.sym"
    # The dSYM itself is archived too: `dump_syms` output is enough for a
    # Breakpad stackwalk, but Apple's own tools (`atos`, Xcode's crash viewer)
    # want the bundle, and the difference matters when someone sends in a
    # `.ips` report from Console rather than a minidump.
    tar -czf "$DESTINATION/$NAME.dSYM.tar.gz" -C "$(dirname "$DSYM")" "$(basename "$DSYM")"
    ;;

  windows-*)
    # MSVC writes the PDB beside the executable. `debug = "line-tables-only"`
    # in the workspace's dev profile does not apply here: release builds need
    # `debug = true` or `split-debuginfo` set, or the PDB is empty of anything
    # useful. See docs/RELEASE.md.
    PDB="${BINARY%.exe}.pdb"
    if [ ! -f "$PDB" ]; then
      echo "no PDB at $PDB — was the release profile built with debug info?" >&2
      exit 1
    fi
    echo "==> dumping symbols from $PDB"
    dump_syms "$PDB" --output "$DESTINATION/$NAME.sym"
    cp "$PDB" "$DESTINATION/"
    ;;

  linux-*)
    echo "==> dumping symbols from $BINARY"
    dump_syms "$BINARY" --output "$DESTINATION/$NAME.sym"
    # Strip afterwards, not before: the shipped binary should not carry the
    # debug info the archive now holds. The caller decides whether to use the
    # stripped copy; this script never modifies the input.
    if command -v objcopy >/dev/null 2>&1; then
      objcopy --only-keep-debug "$BINARY" "$DESTINATION/$NAME.debug"
    fi
    ;;

  *)
    echo "unknown platform: $PLATFORM" >&2
    exit 1
    ;;
esac

# The first line of a Breakpad symbol file is
#   MODULE <os> <arch> <debug id> <name>
# and the debug id is what a minidump carries. Printing it here means the CI log
# records the mapping even if the archive is later misplaced.
echo "==> symbol file header:"
head -n 1 "$DESTINATION/$NAME.sym"

cat > "$DESTINATION/README.txt" <<EOF
Symbols for $NAME
platform : $PLATFORM
version  : $VERSION
build    : $BUILD

A crash report from this build reports symbol_path()
    $PLATFORM/$VERSION/$BUILD
which is this directory.

To symbolicate a minidump against it:
    cargo install minidump-stackwalk
    minidump-stackwalk --symbols-path <root of this archive> crash.dmp
EOF

echo "==> wrote $DESTINATION"
