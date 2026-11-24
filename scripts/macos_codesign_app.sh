#!/bin/sh
# Ad-hoc sign + clear quarantine on a StruggleTime.app bundle.
# Required on modern macOS (Sequoia+): unsigned apps fail LaunchServices with
# RBSRequestErrorDomain Code=5 / NSPOSIXErrorDomain Code=162
# ("Launchd job spawn failed").
#
# Usage: macos_codesign_app.sh <StruggleTime.app>
set -euo pipefail

APP="${1:?path to StruggleTime.app}"
EXE="$APP/Contents/MacOS/struggletime"
PLIST="$APP/Contents/Info.plist"

if [ ! -d "$APP/Contents" ]; then
  echo "not an app bundle: $APP" >&2
  exit 1
fi
if [ ! -f "$EXE" ]; then
  echo "missing CFBundleExecutable binary: $EXE" >&2
  exit 1
fi
chmod 755 "$EXE"

if [ ! -f "$APP/Contents/PkgInfo" ]; then
  printf 'APPL????' >"$APP/Contents/PkgInfo"
fi

if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$PLIST"
fi

if command -v xattr >/dev/null 2>&1; then
  xattr -cr "$APP" 2>/dev/null || true
fi

if ! command -v codesign >/dev/null 2>&1; then
  echo "codesign unavailable — skip signing ($APP)" >&2
  exit 0
fi

# Sign the Mach-O first, then the bundle (deep covers Resources).
codesign --force --sign - "$EXE"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
echo "ad-hoc signed $APP"
