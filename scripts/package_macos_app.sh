#!/bin/sh
# Build StruggleTime.app from a release binary.
# Usage: package_macos_app.sh <struggletime-binary> <out-dir>
set -euo pipefail

BIN="${1:?path to struggletime binary}"
OUT="${2:?output directory}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$OUT/StruggleTime.app"
MACOS="$APP/Contents/MacOS"
RES="$APP/Contents/Resources"
# Prefer workspace Cargo.toml version; override with STRUGGLETIME_VERSION.
APP_VERSION="${STRUGGLETIME_VERSION:-}"
if [ -z "$APP_VERSION" ] && [ -f "$ROOT/rust/Cargo.toml" ]; then
  APP_VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/rust/Cargo.toml" | head -1)"
fi
APP_VERSION="${APP_VERSION:-0.1.4}"

rm -rf "$APP"
mkdir -p "$MACOS" "$RES"
# Native Mach-O only — a shell wrapper makes macOS open Terminal.app.
# Finder launches with no argv; LSEnvironment forces --tray so we never
# depend on TTY heuristics (SSH/CI must not auto-tray).
cp "$BIN" "$MACOS/struggletime"
chmod +x "$MACOS/struggletime"

ICON_SRC="$ROOT/daemon/assets/struggletime.svg"
ICNS_SRC="$ROOT/daemon/assets/AppIcon.icns"
if [ -f "$ICNS_SRC" ]; then
  cp "$ICNS_SRC" "$RES/AppIcon.icns"
elif [ -f "$ICON_SRC" ] && command -v iconutil >/dev/null 2>&1; then
  ICONSET="$(mktemp -d)/AppIcon.iconset"
  mkdir -p "$ICONSET"
  if command -v rsvg-convert >/dev/null 2>&1; then
    TMP_SVG="$(mktemp).svg"
    sed 's/currentColor/#EC3750/g' "$ICON_SRC" > "$TMP_SVG"
    for pair in \
      "16:icon_16x16.png" "32:icon_16x16@2x.png" "32:icon_32x32.png" \
      "64:icon_32x32@2x.png" "128:icon_128x128.png" "256:icon_128x128@2x.png" \
      "256:icon_256x256.png" "512:icon_256x256@2x.png" "512:icon_512x512.png" \
      "1024:icon_512x512@2x.png"
    do
      size="${pair%%:*}"
      name="${pair#*:}"
      rsvg-convert -w "$size" -h "$size" "$TMP_SVG" -o "$ICONSET/$name"
    done
    rm -f "$TMP_SVG"
  elif [ -f "$ROOT/daemon/assets/struggletime-256.png" ] && command -v sips >/dev/null 2>&1; then
    BASE="$ROOT/daemon/assets/struggletime-256.png"
    for pair in \
      "16:icon_16x16.png" "32:icon_16x16@2x.png" "32:icon_32x32.png" \
      "64:icon_32x32@2x.png" "128:icon_128x128.png" "256:icon_128x128@2x.png" \
      "256:icon_256x256.png" "512:icon_256x256@2x.png" "512:icon_512x512.png" \
      "1024:icon_512x512@2x.png"
    do
      size="${pair%%:*}"
      name="${pair#*:}"
      sips -z "$size" "$size" "$BASE" --out "$ICONSET/$name" >/dev/null
    done
  fi
  if [ -n "$(ls -A "$ICONSET" 2>/dev/null || true)" ]; then
    iconutil -c icns "$ICONSET" -o "$RES/AppIcon.icns"
  fi
elif [ -f "$ICON_SRC" ]; then
  cp "$ICON_SRC" "$RES/AppIcon.svg"
fi

{
  cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>struggletime</string>
  <key>CFBundleIdentifier</key>
  <string>com.struggletime.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>StruggleTime</string>
  <key>CFBundleDisplayName</key>
  <string>StruggleTime</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${APP_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${APP_VERSION}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>LSEnvironment</key>
  <dict>
    <key>STRUGGLETIME_APP_BUNDLE</key>
    <string>1</string>
  </dict>
PLIST
  if [ -f "$RES/AppIcon.icns" ]; then
    cat <<'PLIST'
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
PLIST
  fi
  cat <<'PLIST'
</dict>
</plist>
PLIST
} > "$APP/Contents/Info.plist"

echo "built $APP"
