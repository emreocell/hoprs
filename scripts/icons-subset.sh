#!/usr/bin/env bash
# Rebuilds crates/hoprs-app/assets/fonts/Phosphor-hoprs.ttf: Phosphor Regular 2.1.1 (MIT)
# cut down to the glyphs listed in crates/hoprs-app/assets/fonts/phosphor-glyphs.txt.
# Needs `pyftsubset` on PATH and python3 with fontTools (pip install fonttools), curl and
# sha256sum or shasum. Installs nothing. The subset's family is renamed "Phosphor hoprs" so
# it never clashes with a full Phosphor font installed on the system.
# Also refreshes LICENSE-Phosphor.txt from the package.
set -euo pipefail

VERSION=2.1.1
SHA256=06b91e022b7ee899a63efced879392a74f0bacbda54e4467e9f663220d173a10
BASE="https://unpkg.com/@phosphor-icons/web@${VERSION}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fonts="$root/crates/hoprs-app/assets/fonts"

if ! command -v pyftsubset >/dev/null 2>&1; then
    echo "error: pyftsubset not found on PATH; install it with: pip install fonttools" >&2
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$BASE/src/regular/Phosphor.ttf" -o "$tmp/Phosphor.ttf"
curl -fsSL "$BASE/LICENSE" -o "$tmp/LICENSE"

if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/Phosphor.ttf" | cut -d' ' -f1)"
else
    actual="$(shasum -a 256 "$tmp/Phosphor.ttf" | cut -d' ' -f1)"
fi
if [ "$actual" != "$SHA256" ]; then
    echo "error: Phosphor.ttf SHA-256 is $actual, expected $SHA256" >&2
    exit 1
fi

unicodes="$(grep -v '^[[:space:]]*#' "$fonts/phosphor-glyphs.txt" | awk 'NF { printf "%s%s", sep, $2; sep="," }')"

pyftsubset "$tmp/Phosphor.ttf" \
    --unicodes="$unicodes" \
    --layout-features='' \
    --output-file="$fonts/Phosphor-hoprs.ttf"
python3 - "$fonts/Phosphor-hoprs.ttf" <<'PY'
import sys
from fontTools.ttLib import TTFont

font = TTFont(sys.argv[1])
family = "Phosphor hoprs"
names = {1: family, 4: f"{family} Regular", 6: family.replace(" ", "-") + "-Regular", 16: family}
for record in list(font["name"].names):
    if record.nameID in names:
        font["name"].removeNames(nameID=record.nameID)
for name_id, value in names.items():
    font["name"].setName(value, name_id, 3, 1, 0x409)
    font["name"].setName(value, name_id, 1, 0, 0)
font.save(sys.argv[1])
PY
cp "$tmp/LICENSE" "$fonts/LICENSE-Phosphor.txt"
echo "wrote $fonts/Phosphor-hoprs.ttf ($(wc -c <"$fonts/Phosphor-hoprs.ttf") bytes)"
