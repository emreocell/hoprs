#!/usr/bin/env bash
# Builds the hoprs Debian package and prints its path.
# Usage: scripts/package-deb.sh [--container]
#
# Without arguments it builds on this machine: installs cargo-deb (pinned) if missing,
# `cargo build --release -p hoprs-app`, then `cargo deb -p hoprs-app --no-build`. The
# package then needs this machine's glibc, so build it on Ubuntu 24.04 (the oldest system
# the package supports); the script fails if the package would not install there.
#
# With --container it runs the same build inside an Ubuntu 24.04 image (docker/deb-build,
# needs docker and the network), from any host. Its output lands in
# target/deb-ubuntu-24.04/debian/. Package metadata: [package.metadata.deb] in
# crates/hoprs-app/Cargo.toml. Check the result with scripts/check-deb.sh <deb>.
set -euo pipefail

CARGO_DEB_VERSION=3.8.0
# The toolchain the container build pins (overrides rust-toolchain.toml's `stable`).
RUST_VERSION=${HOPRS_DEB_RUST_VERSION:-1.99.1}
# glibc of Ubuntu 24.04: the package must not need a newer one.
MAX_GLIBC=2.39

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

if [ "${1:-}" = "--container" ]; then
    image=hoprs-deb-build:ubuntu-24.04-rust-$RUST_VERSION
    docker build -t "$image" \
        --build-arg RUST_VERSION="$RUST_VERSION" \
        --build-arg CARGO_DEB_VERSION="$CARGO_DEB_VERSION" \
        docker/deb-build >&2
    target=target/deb-ubuntu-24.04
    mkdir -p "$target/cargo-home"
    deb=$(docker run --rm --user "$(id -u):$(id -g)" \
        -v "$repo:/src" -w /src \
        -e HOME=/tmp \
        -e CARGO_HOME="/src/$target/cargo-home" \
        -e CARGO_TARGET_DIR="/src/$target" \
        "$image" scripts/package-deb.sh)
    echo "$repo/${deb#/src/}"
    exit 0
elif [ $# -ne 0 ]; then
    echo "usage: $0 [--container]" >&2
    exit 2
fi

if [ "$(cargo deb --version 2>/dev/null)" != "cargo-deb $CARGO_DEB_VERSION" ]; then
    cargo install cargo-deb --version "$CARGO_DEB_VERSION" --locked >&2
fi
cargo build --release -p hoprs-app >&2
deb=$(cargo deb -p hoprs-app --no-build | tail -n 1)

# `$auto` derives the libc6 version from the glibc symbols the binary uses.
libc=$(dpkg-deb -f "$deb" Depends | sed -n 's/.*libc6 (>= \([^)]*\)).*/\1/p')
if [ -n "$libc" ] && dpkg --compare-versions "$libc" gt "$MAX_GLIBC"; then
    echo "FAIL  $deb needs glibc $libc; Ubuntu 24.04 has $MAX_GLIBC." >&2
    echo "      Build on Ubuntu 24.04, or run: $0 --container" >&2
    exit 1
fi
echo "$deb"
