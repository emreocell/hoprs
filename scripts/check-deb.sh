#!/usr/bin/env bash
# Proves a hoprs .deb installs, runs and uninstalls cleanly on a clean Ubuntu 24.04.
# Usage: scripts/check-deb.sh <path/to/hoprs_X.Y.Z-1_amd64.deb>
#
# Needs docker, and the network inside the container (apt). Builds the image in
# docker/deb-check, then runs docker/deb-check/inside.sh there with the package mounted
# read-only: install with Depends only, `hoprs --version`, desktop-file-validate,
# appstreamcli, every icon size, the start-up probe under xvfb-run (and that every library
# it loaded, dlopen'ed ones included, came from Depends), then remove and purge, checking
# the installed files are gone and the user's ~/.config/hoprs and ~/.local/share/hoprs are
# not. Prints OK/FAIL per stage and exits non-zero if any failed.
set -euo pipefail

image=hoprs-deb-check:ubuntu-24.04
repo=$(cd "$(dirname "$0")/.." && pwd)

if [ $# -ne 1 ]; then
    echo "usage: $0 <path/to/hoprs.deb>" >&2
    exit 2
fi
if [ ! -f "$1" ]; then
    echo "FAIL  package file exists: $1"
    exit 1
fi
echo "OK    package file exists: $1"
deb_dir=$(cd "$(dirname "$1")" && pwd)
deb_name=$(basename "$1")

if docker build -q -t "$image" "$repo/docker/deb-check" >/dev/null; then
    echo "OK    build image $image"
else
    echo "FAIL  build image $image"
    exit 1
fi

docker run --rm -v "$deb_dir:/pkg:ro" "$image" "/pkg/$deb_name"
