#!/bin/sh
# Wrap the built binary in a minimal macOS .app bundle.
# Usage: scripts/bundle.sh [debug|release]
set -e
PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/$PROFILE/opensheet"
APP="$ROOT/target/$PROFILE/OpenSheet.app"
[ -x "$BIN" ] || { echo "build first: cargo build -p opensheet" >&2; exit 1; }
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/opensheet"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>OpenSheet</string>
  <key>CFBundleDisplayName</key><string>OpenSheet</string>
  <key>CFBundleIdentifier</key><string>org.opensheet.OpenSheet</string>
  <key>CFBundleExecutable</key><string>opensheet</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
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
