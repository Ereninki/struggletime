#!/bin/sh
# Stage a release folder with binary + install helpers for one OS artifact.
# Usage: stage_release.sh <artifact-name> <binary-path> <out-dist-dir>
set -euo pipefail

ARTIFACT="${1:?artifact name e.g. struggletime-linux-x86_64}"
BIN="${2:?binary path}"
DIST="${3:?dist dir}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$DIST/$ARTIFACT"

rm -rf "$OUT"
mkdir -p "$OUT"

case "$ARTIFACT" in
  *windows*)
    cp "$BIN" "$OUT/struggletime.exe"
    cp "$ROOT/scripts/install.ps1" "$OUT/install.ps1"
    if [ -f "$ROOT/daemon/assets/struggletime.ico" ]; then
      cp "$ROOT/daemon/assets/struggletime.ico" "$OUT/struggletime.ico"
      mkdir -p "$OUT/daemon/assets"
      cp "$ROOT/daemon/assets/struggletime.ico" "$OUT/daemon/assets/struggletime.ico"
      cp "$ROOT/daemon/assets/struggletime.svg" "$OUT/daemon/assets/struggletime.svg" 2>/dev/null || true
    fi
    if [ -d "$ROOT/adapters" ]; then
      cp -a "$ROOT/adapters" "$OUT/adapters"
    fi
    if [ -d "$ROOT/daemon" ]; then
      mkdir -p "$OUT/daemon"
      cp -f "$ROOT/daemon/hooks.sh" "$ROOT/daemon/hooks.ps1" "$OUT/daemon/" 2>/dev/null || true
    fi
    cat > "$OUT/README.txt" <<'EOF'
StruggleTime for Windows
========================

1. Right-click install.ps1 → Run with PowerShell
   (or: powershell -ExecutionPolicy Bypass -File .\install.ps1)

That installs:
  %LocalAppData%\Programs\StruggleTime\struggletime.exe
  Start Menu → StruggleTime  (tray app)
  user PATH entry so `struggletime` works in new terminals

Optional login autostart:
  powershell -ExecutionPolicy Bypass -File .\install.ps1 -Autostart

Then:
  struggletime --tray        # tray (Start Menu shortcut already uses --tray)
  struggletime --version
  struggletime apikey <st_…>
  struggletime status
EOF
    ;;
  *macos*)
    cp "$BIN" "$OUT/struggletime"
    chmod 755 "$OUT/struggletime"
    sh "$ROOT/scripts/package_macos_app.sh" "$OUT/struggletime" "$OUT"
    cp "$ROOT/scripts/install.sh" "$OUT/install.sh"
    cp "$ROOT/scripts/macos_codesign_app.sh" "$OUT/macos_codesign_app.sh"
    chmod +x "$OUT/install.sh" "$OUT/macos_codesign_app.sh"
    if [ -d "$ROOT/adapters" ]; then
      cp -a "$ROOT/adapters" "$OUT/adapters"
      mkdir -p "$OUT/StruggleTime.app/Contents/Resources"
      cp -a "$ROOT/adapters" "$OUT/StruggleTime.app/Contents/Resources/adapters"
      # Re-sign after mutating Resources (invalidates prior signature).
      sh "$ROOT/scripts/macos_codesign_app.sh" "$OUT/StruggleTime.app"
    fi
    if [ -d "$ROOT/daemon" ]; then
      mkdir -p "$OUT/daemon"
      cp -f "$ROOT/daemon/hooks.sh" "$ROOT/daemon/hooks.ps1" "$OUT/daemon/" 2>/dev/null || true
    fi
    cat > "$OUT/README.txt" <<'EOF'
StruggleTime for macOS
======================

Option A — App + CLI installer (preferred):
  sh ./install.sh ./struggletime

Copies StruggleTime.app to /Applications (or ~/Applications), clears quarantine,
ad-hoc code-signs the bundle (fixes RBSRequestErrorDomain / POSIX 162), and
symlinks `struggletime` into ~/.local/bin (and /usr/local/bin when writable).

Option B — manual:
  xattr -cr StruggleTime.app
  sh ./macos_codesign_app.sh StruggleTime.app
  open StruggleTime.app          # menu-bar tray (no Dock icon)
  sudo ln -sf "$(pwd)/StruggleTime.app/Contents/MacOS/struggletime" /usr/local/bin/struggletime

Then:
  struggletime --version
  struggletime apikey <st_…>
  struggletime status
EOF
    ;;
  *linux*)
    cp "$BIN" "$OUT/struggletime"
    chmod 755 "$OUT/struggletime"
    cp "$ROOT/scripts/install.sh" "$OUT/install.sh"
    chmod +x "$OUT/install.sh"
    mkdir -p "$OUT/share/icons"
    if [ -f "$ROOT/daemon/assets/struggletime.svg" ]; then
      cp "$ROOT/daemon/assets/struggletime.svg" "$OUT/share/icons/struggletime.svg"
    fi
    if [ -f "$ROOT/daemon/assets/struggletime.ico" ]; then
      cp "$ROOT/daemon/assets/struggletime.ico" "$OUT/share/icons/struggletime.ico"
    fi
    if [ -f "$ROOT/daemon/assets/AppIcon.icns" ]; then
      mkdir -p "$OUT/daemon/assets"
      cp "$ROOT/daemon/assets/AppIcon.icns" "$OUT/daemon/assets/AppIcon.icns"
      cp "$ROOT/daemon/assets/struggletime.svg" "$OUT/daemon/assets/struggletime.svg" 2>/dev/null || true
    fi
    if [ -d "$ROOT/adapters" ]; then
      cp -a "$ROOT/adapters" "$OUT/adapters"
    fi
    if [ -d "$ROOT/daemon" ]; then
      mkdir -p "$OUT/daemon"
      cp -f "$ROOT/daemon/hooks.sh" "$ROOT/daemon/hooks.ps1" "$OUT/daemon/" 2>/dev/null || true
    fi
    cat > "$OUT/struggletime.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=StruggleTime
Comment=Track struggle-weighted coding time
Exec=struggletime --tray
Icon=struggletime
Terminal=false
Categories=Development;Utility;
StartupNotify=false
EOF
    if command -v desktop-file-validate >/dev/null 2>&1; then
      desktop-file-validate "$OUT/struggletime.desktop" || true
    fi
    cat > "$OUT/README.txt" <<'EOF'
StruggleTime for Linux
======================

  sh ./install.sh ./struggletime

Installs:
  ~/.local/bin/struggletime
  ~/.local/share/applications/struggletime.desktop
  optional ~/.config/autostart/struggletime.desktop

Release builds include the AppIndicator tray. Runtime libs: libxdo, GTK3, and
libayatana-appindicator3 (or libappindicator). Without them, StruggleTime still
runs a headless supervisor. With no DISPLAY/WAYLAND, `struggletime --tray`
fails fast (use --headless for supervisor-only).

Then:
  struggletime --tray       # tray
  struggletime --version
  struggletime apikey <st_…>
  struggletime status
EOF
    ;;
  *)
    cp "$BIN" "$OUT/struggletime"
    chmod +x "$OUT/struggletime" 2>/dev/null || true
    ;;
esac

echo "staged $OUT"
