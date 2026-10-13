#!/bin/sh
# Build the helper through which OpenSheet reaches the translation built
# into macOS (Review > Translate): crates/app/helpers/translate.swift,
# compiled next to the opensheet binary as "opensheet-translate".
# Usage: scripts/build_translate_helper.sh [debug|release]
#
# Needs the Swift compiler of Xcode or of the Command Line Tools. Without
# it, or on a macOS whose SDK has no Translation framework, nothing is
# built and the Translator pane says translation is not available.
PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/app/helpers/translate.swift"
OUT="$ROOT/target/$PROFILE/opensheet-translate"
if ! command -v swiftc >/dev/null 2>&1; then
  echo "translate helper: no Swift compiler (swiftc), skipped" >&2
  exit 0
fi
mkdir -p "$ROOT/target/$PROFILE"
[ "$PROFILE" = release ] && OPT=-O || OPT=-Onone
# An old deployment target: the helper starts on any macOS OpenSheet runs
# on, and answers "unsupported-system" where the framework is missing.
if swiftc $OPT -parse-as-library -target "$(uname -m)-apple-macos13.0" "$SRC" -o "$OUT" 2>"$OUT.log"; then
  rm -f "$OUT.log"
  echo "$OUT"
else
  echo "translate helper: build failed, skipped (see $OUT.log)" >&2
  rm -f "$OUT"
fi
exit 0
