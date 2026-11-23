#!/bin/sh
# Regenerate desktop app icons (ICO / ICNS / PNG) from struggletime.svg.
# Requires: rsvg-convert, png2icns (icnsutils), python3 + Pillow
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SVG="$ROOT/daemon/assets/struggletime.svg"
OUT="$ROOT/daemon/assets"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

test -f "$SVG"
command -v rsvg-convert >/dev/null
command -v png2icns >/dev/null
python3 -c 'from PIL import Image' >/dev/null

sed 's/currentColor/#EC3750/g' "$SVG" > "$TMP/app.svg"
mkdir -p "$TMP/png"
for s in 16 32 48 64 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" "$TMP/app.svg" -o "$TMP/png/icon-$s.png"
done

png2icns "$OUT/AppIcon.icns" \
  "$TMP/png/icon-16.png" \
  "$TMP/png/icon-32.png" \
  "$TMP/png/icon-128.png" \
  "$TMP/png/icon-256.png" \
  "$TMP/png/icon-512.png"

cp "$TMP/png/icon-256.png" "$OUT/struggletime-256.png"

python3 <<PY
import struct
from io import BytesIO
from pathlib import Path
from PIL import Image

tmp = Path("$TMP/png")
out = Path("$OUT/struggletime.ico")
sizes = [16, 32, 48, 64, 128, 256]
blobs, entries = [], []
for s in sizes:
    im = Image.open(tmp / f"icon-{s}.png").convert("RGBA")
    buf = BytesIO()
    im.save(buf, format="PNG")
    b = buf.getvalue()
    entries.append((s, len(b)))
    blobs.append(b)
header = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
dirents = b""
for s, n in entries:
    w = 0 if s >= 256 else s
    dirents += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, n, offset)
    offset += n
out.write_bytes(header + dirents + b"".join(blobs))
print("wrote", out)
PY

mkdir -p "$ROOT/rust/crates/struggletime/assets"
cp "$OUT/struggletime.ico" "$ROOT/rust/crates/struggletime/assets/struggletime.ico"
cp "$SVG" "$ROOT/rust/crates/struggletime/assets/struggletime.svg"
echo "wrote $OUT/AppIcon.icns $OUT/struggletime.ico $OUT/struggletime-256.png"
