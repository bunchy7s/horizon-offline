#!/usr/bin/env bash
# Packages the macOS launcher: Horizon.app (the binary, Info.plist, the icon), signed, then the
# two release assets, horizon-launcher-macos-ARCH.zip (what the launcher updates itself from;
# older launchers find the binary inside it) and horizon-launcher-macos-ARCH.dmg (the installer:
# the app and an Applications link over a background, to drag one onto the other). ARCH is the
# binary's: arm64 (Apple Silicon, the default) or x64 (Intel).
#
#   packaging/macos/package.sh BINARY VERSION OUTDIR [arm64|x64]
#
# Needs macOS (ditto, codesign, hdiutil) and dmgbuild (pip install dmgbuild==1.6.5; $DMGBUILD
# names it if it isn't on PATH). Signing: ad hoc by default, which needs no Apple account. With
# MACOS_SIGN_IDENTITY set to a "Developer ID Application: …" identity (in $MACOS_SIGN_KEYCHAIN
# when set), the app and the .dmg are signed with the hardened runtime and a timestamp, and with
# MACOS_NOTARY_PROFILE too, both are notarized and stapled (keychain.sh unlock sets the two;
# see packaging/macos/README.md).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
binary=$1
version=$2
out=$(mkdir -p "$3" && cd "$3" && pwd)
arch=${4:-arm64}
case "$arch" in
  arm64) cpu=arm64 ;;
  x64) cpu=x86_64 ;;
  *) echo "unknown architecture $arch (arm64 or x64)" >&2; exit 1 ;;
esac
# The asset names say which Macs the binary runs on: it must be built for that processor.
[[ "$(lipo -archs "$binary")" == "$cpu" ]] || { echo "$binary is $(lipo -archs "$binary"), not $cpu" >&2; exit 1; }
zip=horizon-launcher-macos-$arch.zip
dmg=$out/horizon-launcher-macos-$arch.dmg
app=$out/Horizon.app

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/horizon-launcher"
chmod 755 "$app/Contents/MacOS/horizon-launcher"
cp "$here/../icons/horizon.icns" "$app/Contents/Resources/horizon.icns"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist"

identity=${MACOS_SIGN_IDENTITY:--}
keychain=()
[[ -n "${MACOS_SIGN_KEYCHAIN:-}" ]] && keychain=(--keychain "$MACOS_SIGN_KEYCHAIN")
if [[ "$identity" == "-" ]]; then
  codesign --force --sign - "$app"
else
  codesign --force --options runtime --timestamp ${keychain[@]+"${keychain[@]}"} --sign "$identity" "$app"
fi
codesign --verify --strict --verbose=2 "$app"
# Still starts under the hardened runtime.
"$app/Contents/MacOS/horizon-launcher" --version
notarize=$([[ "$identity" != "-" && -n "${MACOS_NOTARY_PROFILE:-}" ]] && echo yes || true)
# The app first, so the zip and the .dmg both carry it stapled.
[[ -z "$notarize" ]] || "$here/notarize.sh" "$app"

rm -f "$out/$zip" "$dmg"
# No resource forks or extended attributes: they would land as ._ files inside the bundle.
(cd "$out" && ditto -c -k --norsrc --noextattr --noqtn --keepParent Horizon.app "$zip")

cat > "$out/dmg.json" <<JSON
{
  "title": "Horizon",
  "icon-size": 128,
  "background": "$here/background.png",
  "window": { "position": { "x": 200, "y": 160 }, "size": { "width": 660, "height": 420 } },
  "format": "UDZO",
  "contents": [
    { "x": 180, "y": 200, "type": "file", "path": "$app" },
    { "x": 480, "y": 200, "type": "link", "path": "/Applications" }
  ]
}
JSON
"${DMGBUILD:-dmgbuild}" -s "$out/dmg.json" Horizon "$dmg"
rm "$out/dmg.json"
if [[ "$identity" != "-" ]]; then
  codesign --force --timestamp ${keychain[@]+"${keychain[@]}"} --sign "$identity" "$dmg"
  codesign --verify --strict --verbose=2 "$dmg"
fi
hdiutil verify "$dmg" >/dev/null
if [[ -n "$notarize" ]]; then
  "$here/notarize.sh" "$dmg"
  # What Gatekeeper says of a download: both must be "source=Notarized Developer ID".
  spctl --assess --verbose=2 "$app"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
fi
echo "Packaged $out/$zip and $dmg"
