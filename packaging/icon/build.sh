#!/bin/sh
# Regenerates every icon file of the repository from icon.py. Runs on macOS
# (AppKit draws the SVGs; iconutil and Xcode's actool make the macOS files).
#   macOS 26 and later: packaging/macos/Assets.car, from AppIcon.icon (Icon Composer)
#   earlier macOS:      packaging/macos/AppIcon.icns
#   Windows:            packaging/windows/opensheet.ico
#   Linux:              packaging/linux/icons/hicolor/*/apps/org.opensheet.OpenSheet.*
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
swiftc -O "$HERE/render.swift" -o "$WORK/render"
for v in flat small mac mac-small; do python3 "$HERE/icon.py" "$v" > "$WORK/$v.svg"; done
python3 "$HERE/icon.py" layer-sheet > "$HERE/AppIcon.icon/Assets/sheet.svg"
python3 "$HERE/icon.py" layer-selection > "$HERE/AppIcon.icon/Assets/selection.svg"
# The small drawing up to 32 pixels, the detailed one above.
png() { if [ "$2" -le 32 ]; then src="$1-small"; else src="$1"; fi; "$WORK/render" "$WORK/$src.svg" "$3" "$2"; }

# Linux
LINUX="$ROOT/packaging/linux/icons/hicolor"
rm -rf "$LINUX"
mkdir -p "$LINUX/scalable/apps"
cp "$WORK/flat.svg" "$LINUX/scalable/apps/org.opensheet.OpenSheet.svg"
cp "$WORK/small.svg" "$WORK/flat-small.svg"
for n in 16 24 32 48 64 128 256 512; do
  mkdir -p "$LINUX/${n}x${n}/apps"
  png flat "$n" "$LINUX/${n}x${n}/apps/org.opensheet.OpenSheet.png"
done

# Windows
for n in 16 24 32 48 64 128 256; do png flat "$n" "$WORK/win$n.png"; done
python3 "$HERE/ico.py" "$ROOT/packaging/windows/opensheet.ico" \
  "$WORK/win16.png" "$WORK/win24.png" "$WORK/win32.png" "$WORK/win48.png" \
  "$WORK/win64.png" "$WORK/win128.png" "$WORK/win256.png"

# macOS before 26
SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"
for n in 16 32 128 256 512; do
  png mac "$n" "$SET/icon_${n}x${n}.png"
  png mac "$((n * 2))" "$SET/icon_${n}x${n}@2x.png"
done
iconutil -c icns "$SET" -o "$ROOT/packaging/macos/AppIcon.icns"

# macOS 26 and later
xcrun actool "$HERE/AppIcon.icon" --compile "$WORK" --output-format human-readable-text \
  --errors --warnings --output-partial-info-plist "$WORK/partial.plist" --app-icon AppIcon \
  --include-all-app-icons --enable-on-demand-resources NO --development-region en \
  --target-device mac --minimum-deployment-target 12.0 --platform macosx > /dev/null
cp "$WORK/Assets.car" "$ROOT/packaging/macos/Assets.car"
echo "icons written"
