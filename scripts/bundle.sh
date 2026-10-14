#!/bin/sh
# Wrap the built binary in a minimal macOS .app bundle.
# Usage: scripts/bundle.sh [debug|release] [target triple]
# With a target triple (as built by `cargo build --target`), the binary and
# the bundle are those of target/<triple>/<profile>.
# OPENSHEET_BUNDLE_ID gives the bundle another identifier, so that a copy
# driven by automated tests is never mistaken for the one the user has open.
set -e
PROFILE="${1:-debug}"
TARGET="${2:-}"
BUNDLE_ID="${OPENSHEET_BUNDLE_ID:-org.opensheet.OpenSheet}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="$ROOT/target/${TARGET:+$TARGET/}$PROFILE"
BIN="$DIR/opensheet"
APP="$DIR/OpenSheet.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
[ -x "$BIN" ] || { echo "build first: cargo build -p opensheet" >&2; exit 1; }
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/opensheet"
# The helper of Review > Translate goes next to the binary, when the Swift
# compiler is there to build it (the pane says so when it is missing).
"$ROOT/scripts/build_translate_helper.sh" "$PROFILE" "$TARGET" >/dev/null
HELPER="$DIR/opensheet-translate"
if [ -x "$HELPER" ]; then cp "$HELPER" "$APP/Contents/MacOS/opensheet-translate"; fi
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>OpenSheet</string>
  <key>CFBundleDisplayName</key><string>OpenSheet</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>opensheet</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Spreadsheet</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>CFBundleTypeExtensions</key>
      <array><string>xlsx</string><string>xlsm</string><string>csv</string><string>tsv</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
echo "$APP"
