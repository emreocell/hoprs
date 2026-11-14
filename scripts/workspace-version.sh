#!/usr/bin/env bash
# Prints [workspace.package] version from Cargo.toml at the repository root.
# Exits non-zero with a message if the section or the key is missing.
# Used by .github/workflows/release.yml (tag check) and ci.yml (CHANGELOG check).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(awk '
    { sub(/\r$/, "") }
    /^\[/ { section = $0; next }
    section == "[workspace.package]" && /^version *=/ {
        gsub(/^version *= *"|".*$/, ""); print; exit
    }' "$repo/Cargo.toml")
if [ -z "$version" ]; then
    echo "workspace-version: no version in [workspace.package] of $repo/Cargo.toml" >&2
    exit 1
fi
echo "$version"
