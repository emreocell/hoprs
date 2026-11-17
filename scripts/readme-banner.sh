#!/usr/bin/env bash
# Renders the README header image, docs/images/banner.png, from docs/images/banner.html
# with headless Chrome (1280x640 CSS px at device scale 2, so 2560x1280). The page uses the
# app's fonts and logo and the README screenshots; nothing is fetched from the network.
#
#   scripts/readme-banner.sh            # Chrome from PATH
#   CHROME=/path/to/chrome scripts/readme-banner.sh
#
# Run scripts/readme-shots.sh first when the screenshots change.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
page="$root/docs/images/banner.html"
out="$root/docs/images/banner.png"

chrome=${CHROME:-}
if [ -z "$chrome" ]; then
  for candidate in google-chrome google-chrome-stable chromium chromium-browser; do
    if command -v "$candidate" >/dev/null 2>&1; then
      chrome=$candidate
      break
    fi
  done
fi
if [ -z "$chrome" ]; then
  echo "readme-banner: no Chrome or Chromium found; set CHROME=/path/to/chrome" >&2
  exit 1
fi

profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT

"$chrome" --headless=new --disable-gpu --hide-scrollbars --no-first-run \
  --user-data-dir="$profile" \
  --force-device-scale-factor=2 --window-size=1280,640 \
  --screenshot="$out" "file://$page" >/dev/null 2>&1

echo "$out"
