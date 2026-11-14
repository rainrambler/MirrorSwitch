#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
if [[ -z "$VERSION" ]]; then
    echo "error: unable to read package version from Cargo.toml" >&2
    exit 1
fi
DIST="$ROOT/dist"
APP="$DIST/MirrorSwitch.app"
STAGE="$DIST/macos"
BINARY="$ROOT/target/release/MirrorSwitch"

# UNIVERSAL=1 builds both Apple Silicon and Intel variants when the Rust
# targets are installed, then merges them with lipo.
if [[ "${UNIVERSAL:-0}" == "1" ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p "$(dirname "$BINARY")"
    lipo -create \
        "$ROOT/target/aarch64-apple-darwin/release/MirrorSwitch" \
        "$ROOT/target/x86_64-apple-darwin/release/MirrorSwitch" \
        -output "$BINARY"
else
    cargo build --release
fi

rm -rf "$APP" "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/MirrorSwitch"
chmod +x "$APP/Contents/MacOS/MirrorSwitch"
cp "$ROOT/packaging/macos/Info.plist" "$APP/Contents/Info.plist"
if [[ -x /usr/libexec/PlistBuddy ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VERSION" "$APP/Contents/Info.plist"
fi

if command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    ICONSET="$DIST/MirrorSwitch.iconset"
    rm -rf "$ICONSET"
    mkdir -p "$ICONSET"
    for spec in \
        "16 icon_16x16.png" \
        "32 icon_16x16@2x.png" \
        "32 icon_32x32.png" \
        "64 icon_32x32@2x.png" \
        "128 icon_128x128.png" \
        "256 icon_128x128@2x.png" \
        "256 icon_256x256.png" \
        "512 icon_256x256@2x.png" \
        "512 icon_512x512.png" \
        "1024 icon_512x512@2x.png"
    do
        set -- $spec
        sips -z "$1" "$1" "$ROOT/assets/icon-1024.png" --out "$ICONSET/$2" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/MirrorSwitch.icns"
    rm -rf "$ICONSET"
else
    echo "warning: sips/iconutil not found; packaging without an app icon" >&2
fi

# Ad-hoc signing is enough for local distribution. For notarized distribution,
# replace "-" with a Developer ID Application identity.
if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
fi

mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
cp "$ROOT/README.md" "$STAGE/README.md"
cp "$ROOT/LICENSE" "$STAGE/LICENSE"
cp -R "$ROOT/docs" "$STAGE/docs"

ZIP="$DIST/MirrorSwitch-v${VERSION}-macos.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$STAGE" "$ZIP"
(
    cd "$DIST"
    shasum -a 256 "$(basename "$ZIP")" | tee SHA256SUMS-macos.txt
)

echo "Built: $ZIP"
echo "Tip: run 'open dist/MirrorSwitch.app' to launch."
