#!/bin/sh
# Regenerates the app icon PNGs from the SVG, each rendered directly at its size (never
# scaled down from a bigger PNG, so the small ones stay crisp).
# Uses a local rsvg-convert if there is one, else a throwaway Debian container.
# Also packs hoprs.ico (16-256) and hoprs.icns (16-1024) for Windows and macOS with
# scripts/pack-icons.py: python3, standard library only, no macOS or icnsutils tools.
# The 1024 px render exists only to feed the .icns and is deleted afterwards.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dir=crates/hoprs-app/assets/icon
sizes="16 24 32 48 64 128 256 512 1024"

render='svg=$1; dir=$2; shift 2; for n in "$@"; do rsvg-convert -w "$n" -h "$n" -o "$dir/hoprs-$n.png" "$svg"; done'

cd "$root"
if command -v rsvg-convert >/dev/null 2>&1; then
    # shellcheck disable=SC2086
    sh -c "$render" sh "$dir/hoprs.svg" "$dir" $sizes
else
    # shellcheck disable=SC2086
    docker run --rm -v "$root":/w -w /w debian:stable-slim sh -c \
        'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq librsvg2-bin >/dev/null 2>&1 && sh -c "$0" sh "$@" && chown -R $(stat -c %u:%g /w) /w/crates/hoprs-app/assets/icon' \
        "$render" "$dir/hoprs.svg" "$dir" $sizes
fi

python3 scripts/pack-icons.py "$dir"
rm -f "$dir/hoprs-1024.png"
