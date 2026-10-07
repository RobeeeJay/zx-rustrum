#!/usr/bin/env bash
#
# Build ZX-Rustrum.app, and a .dmg holding it.
#
# Signing and notarisation are done when what they need is there, so a machine
# without the certificate still builds an app it can run itself:
#
#   MACOS_SIGN_IDENTITY   a Developer ID Application identity in the keychain
#   MACOS_NOTARY_KEY      path to an App Store Connect API key (.p8), with
#   MACOS_NOTARY_KEY_ID   that key's id and
#   MACOS_NOTARY_ISSUER   its issuer id
#
# MACOS_REQUIRE_SIGNED=true, which the release workflow sets, makes leaving
# any of them out an error rather than a note: a release is never shipped
# unsigned because a secret went missing.
#
# Usage: packaging/macos-app.sh [target-triple]
#        packaging/macos-app.sh --dmg-name [target-triple]   (print it and stop)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"
name_only=""
if [ "${1:-}" = "--dmg-name" ]; then
    name_only=yes
    shift
fi
target="${1:-}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"

# The disk image carries the version and the architecture. Both macOS builds
# are made in the same workflow and their files end up in one directory: two
# images called the same thing would be one image, and whichever was uploaded
# second would be the release.
case "$target" in
    aarch64-*) arch=arm64 ;;
    x86_64-*)  arch=x86_64 ;;
    "")        arch="$(uname -m)" ;;
    *)         arch="${target%%-*}" ;;
esac
dmg="target/packaging/ZX-Rustrum-$version-macos-$arch.dmg"
if [ -n "$name_only" ]; then
    echo "$dmg"
    exit 0
fi

notarising=""
if [ -n "${MACOS_NOTARY_KEY:-}" ] && [ -n "${MACOS_NOTARY_KEY_ID:-}" ] &&
   [ -n "${MACOS_NOTARY_ISSUER:-}" ]; then
    notarising=yes
fi
# Checked before the build rather than after it: a release that cannot be
# signed should say so in seconds, not at the end of a ten-minute build.
if [ "${MACOS_REQUIRE_SIGNED:-}" = true ]; then
    if [ -z "${MACOS_SIGN_IDENTITY:-}" ]; then
        echo "error: a release has to be signed, and MACOS_SIGN_IDENTITY is not set" >&2
        exit 1
    fi
    if [ -z "$notarising" ]; then
        echo "error: a release has to be notarised, and the notary key is not set" >&2
        exit 1
    fi
fi

cd "$root"
if [ -n "$target" ]; then
    cargo build --release --target "$target"
    binary="target/$target/release/zx-rustrum"
else
    cargo build --release
    binary="target/release/zx-rustrum"
fi

app="target/packaging/ZX-Rustrum.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/roms"
cp "$binary" "$app/Contents/MacOS/ZX-Rustrum"

# The icon: macOS wants an .icns, which iconutil builds from a directory of
# PNGs at fixed sizes.
iconset="target/packaging/icon.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size "$here/icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$here/icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/ZX-Rustrum.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>ZX-Rustrum</string>
    <key>CFBundleDisplayName</key><string>ZX-Rustrum</string>
    <key>CFBundleExecutable</key><string>ZX-Rustrum</string>
    <key>CFBundleIdentifier</key><string>com.robeesworld.zx-rustrum</string>
    <key>CFBundleIconFile</key><string>ZX-Rustrum</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Tape, snapshot or ROM image</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSItemContentTypes</key><array><string>public.data</string></array>
            <key>CFBundleTypeExtensions</key>
            <array>
                <string>tzx</string><string>tap</string>
                <string>p</string><string>81</string><string>p81</string>
                <string>sna</string><string>z80</string>
                <string>rom</string><string>bin</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# ROMs are copyrighted and are not shipped; the directory tells the user where
# theirs should go, and the app searches it (see src/resources.rs).
cat > "$app/Contents/Resources/roms/README.txt" <<'TXT'
Put ROM images here:

  48.rom     16K   ZX Spectrum 48K
  128.rom    32K   ZX Spectrum 128K
  plus3.rom  64K   ZX Spectrum +2A/+3
  zx81.rom    8K   ZX81

None are included: they are still under copyright. The emulator also looks
in ~/Library/Application Support/ZX Spectrum Emulator, which survives an
upgrade of the app, and in whichever directory you last opened a ROM from.
TXT

sign() {
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$@"
}

notary() {
    xcrun notarytool "$@" --key "$MACOS_NOTARY_KEY" \
        --key-id "$MACOS_NOTARY_KEY_ID" --issuer "$MACOS_NOTARY_ISSUER"
}

# Send something to Apple and wait for the answer. The status is read from
# what notarytool reports rather than left to its exit status, and Apple's
# log is printed when it was not accepted, since that is the only place that
# says why.
notarise() {
    local result id status
    result="$(notary submit "$1" --wait --timeout 30m --output-format json || true)"
    id="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id", ""))' 2>/dev/null || true)"
    status="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("status", ""))' 2>/dev/null || true)"
    echo "notarisation of $(basename "$1"): ${status:-no answer} ${id:+($id)}"
    if [ "$status" != Accepted ]; then
        printf '%s\n' "$result" >&2
        if [ -n "$id" ]; then
            notary log "$id" >&2 || true
        fi
        exit 1
    fi
}

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    # The hardened runtime is what notarisation asks for. Nothing in the app
    # needs an exception to it: there is no microphone input, no code made at
    # run time, nothing loaded by name. A single executable, so no --deep.
    sign --options runtime "$app"
    codesign --verify --strict --verbose=2 "$app"
    if [ -n "$notarising" ]; then
        # Notarised, and the ticket stapled to the app, before it goes into
        # the image: an app copied out of the image then opens without asking
        # Apple, offline too.
        zip="target/packaging/ZX-Rustrum.zip"
        rm -f "$zip"
        ditto -c -k --keepParent "$app" "$zip"
        notarise "$zip"
        rm -f "$zip"
        xcrun stapler staple "$app"
    fi
else
    echo "note: unsigned. Set MACOS_SIGN_IDENTITY to sign." >&2
fi

rm -f "$dmg"
staging="target/packaging/dmg"
rm -rf "$staging"
mkdir -p "$staging"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "ZX-Rustrum" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    sign "$dmg"
    if [ -n "$notarising" ]; then
        notarise "$dmg"
        xcrun stapler staple "$dmg"
        # What Gatekeeper on another machine will make of them.
        spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
        spctl --assess --type execute --verbose=2 "$app"
    fi
fi

echo "built $app"
echo "built $dmg"
