#!/bin/sh
# Turns the OpenSheet.app of a release build into dist/opensheet-<target>.dmg.
# Usage: packaging/macos/package.sh <target triple>   (after scripts/bundle.sh release <target>)
#
# With a Developer ID certificate in the environment, the app and the disk
# image are signed with the hardened runtime, then notarized by Apple and
# stapled: the app opens with a double-click. Without, the app keeps an
# ad-hoc signature and macOS asks for right-click ▸ Open the first time.
#   MACOS_CERTIFICATE           the .p12 of a "Developer ID Application" certificate, in base64
#   MACOS_CERTIFICATE_PASSWORD  the password of that .p12
#   APPLE_API_KEY               an App Store Connect API key (the text of its .p8), to notarize
#   APPLE_API_KEY_ID            the key's ID
#   APPLE_API_ISSUER            the issuer ID shown above the keys in App Store Connect
set -e
TARGET="$1"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="$ROOT/target/$TARGET/release/OpenSheet.app"
DMG="$ROOT/dist/opensheet-$TARGET.dmg"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
[ -d "$APP" ] || { echo "bundle first: scripts/bundle.sh release $TARGET" >&2; exit 1; }

if [ -n "$MACOS_CERTIFICATE" ]; then
  # A keychain of its own for the certificate, gone with the runner.
  KEYCHAIN="$WORK/signing.keychain-db"
  PASSWORD="$(uuidgen)"
  security create-keychain -p "$PASSWORD" "$KEYCHAIN"
  security set-keychain-settings -lut 3600 "$KEYCHAIN"
  security unlock-keychain -p "$PASSWORD" "$KEYCHAIN"
  printf '%s' "$MACOS_CERTIFICATE" | base64 --decode > "$WORK/certificate.p12"
  security import "$WORK/certificate.p12" -k "$KEYCHAIN" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple: -s -k "$PASSWORD" "$KEYCHAIN" > /dev/null
  # shellcheck disable=SC2046
  security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
  IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" \
    | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)"
  [ -n "$IDENTITY" ] || { echo "no Developer ID Application identity in MACOS_CERTIFICATE" >&2; exit 1; }
  echo "signing with $IDENTITY"
  sign() { codesign --force --options runtime --timestamp --sign "$IDENTITY" "$@"; }
else
  IDENTITY=""
  echo "no MACOS_CERTIFICATE: ad-hoc signature, not notarized"
  sign() { codesign --force --sign - "$@"; }
fi

# Inside out: the translation helper, then the app.
if [ -f "$APP/Contents/MacOS/opensheet-translate" ]; then sign "$APP/Contents/MacOS/opensheet-translate"; fi
sign "$APP"
codesign --verify --strict --deep "$APP"

# The disk image: the app and a link to Applications to drag it onto.
mkdir -p "$WORK/stage" "$ROOT/dist"
cp -R "$APP" "$WORK/stage/"
ln -s /Applications "$WORK/stage/Applications"
# hdiutil now and then fails with "Resource busy" on CI machines.
for attempt in 1 2 3; do
  if hdiutil create -volname OpenSheet -srcfolder "$WORK/stage" -fs HFS+ -format UDZO -ov "$DMG"; then break; fi
  [ "$attempt" = 3 ] && exit 1
  sleep 5
done

if [ -n "$IDENTITY" ]; then
  codesign --force --timestamp --sign "$IDENTITY" "$DMG"
  if [ -n "$APPLE_API_KEY" ]; then
    printf '%s\n' "$APPLE_API_KEY" > "$WORK/key.p8"
    xcrun notarytool submit "$DMG" --key "$WORK/key.p8" --key-id "$APPLE_API_KEY_ID" \
      --issuer "$APPLE_API_ISSUER" --wait --timeout 30m --output-format json > "$WORK/notary.json" || true
    cat "$WORK/notary.json"; echo
    if [ "$(plutil -extract status raw -o - "$WORK/notary.json" 2>/dev/null)" != "Accepted" ]; then
      id="$(plutil -extract id raw -o - "$WORK/notary.json" 2>/dev/null || true)"
      [ -n "$id" ] && xcrun notarytool log "$id" --key "$WORK/key.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER"
      echo "notarization failed" >&2
      exit 1
    fi
    xcrun stapler staple "$DMG"
  else
    echo "no APPLE_API_KEY: signed but not notarized"
  fi
fi
echo "$DMG"
