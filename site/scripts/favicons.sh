#!/bin/sh
# Regenerates site/public/favicon.ico (16 and 32 px) and apple-touch-icon.png (180 px) from
# public/icon.svg (a copy of crates/hoprs-app/assets/icon/hoprs.svg), each size rendered directly.
# Same approach as scripts/icons.sh in the repo root: a local rsvg-convert if there is one,
# else a throwaway Debian container. The .ico just wraps the PNGs (python3, standard library).
set -eu

site=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp "$site/public/icon.svg" "$tmp/icon.svg"

render='cd "$1"; for n in 16 32 180; do rsvg-convert -w "$n" -h "$n" -o "icon-$n.png" icon.svg; done'
if command -v rsvg-convert >/dev/null 2>&1; then
    sh -c "$render" sh "$tmp"
else
    docker run --rm -v "$tmp":/w debian:stable-slim sh -c \
        'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq librsvg2-bin >/dev/null 2>&1 && sh -c "$0" sh /w && chown -R $(stat -c %u:%g /w) /w' \
        "$render"
fi

cp "$tmp/icon-180.png" "$site/public/apple-touch-icon.png"
python3 - "$tmp" "$site/public/favicon.ico" <<'PY'
import struct, sys
from pathlib import Path
d, out = Path(sys.argv[1]), Path(sys.argv[2])
sizes = [16, 32]
frames = [(d / f"icon-{n}.png").read_bytes() for n in sizes]
offset = 6 + 16 * len(sizes)
head, body = struct.pack("<HHH", 0, 1, len(sizes)), b""
for n, data in zip(sizes, frames):
    head += struct.pack("<BBBBHHII", n, n, 0, 0, 1, 32, len(data), offset + len(body))
    body += data
out.write_bytes(head + body)
PY
echo "wrote public/favicon.ico and public/apple-touch-icon.png"
