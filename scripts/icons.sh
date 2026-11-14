#!/bin/sh
# Regenerates the app icon PNGs from the SVG, each rendered directly at its size (never
# scaled down from a bigger PNG, so the small ones stay crisp).
# Uses a local rsvg-convert if there is one, else a throwaway Debian container.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dir=crates/hoprs-app/assets/icon
sizes="16 24 32 48 64 128 256 512"

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
