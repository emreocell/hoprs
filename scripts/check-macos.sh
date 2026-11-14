#!/usr/bin/env bash
# Proves a hoprs macOS disk image holds a working universal app.
# Usage: scripts/check-macos.sh <path/to/hoprs_X.Y.Z_macos_universal.dmg>   (on macOS)
#
# Verifies the image's checksums, mounts it read-only (no Finder window), then checks:
# the Applications link; the binary has both slices (arm64, x86_64), each built for
# macOS 11; the ad-hoc signature verifies; Info.plist (identifier, version = the
# workspace version, LSMinimumSystemVersion, the executable, the icon it names is
# there and iconutil reads it); `hoprs --version` prints `hoprs <version>` (the Intel
# slice too, through Rosetta where the machine is Apple silicon); no builder paths in the
# binary. Detaches the image on exit. Prints OK/FAIL per stage and exits non-zero if any
# failed.
# The checks are functions called through `stage` and the trap: shellcheck takes them
# for unreachable code after the early exits.
# shellcheck disable=SC2317
set -euo pipefail

BUNDLE_ID=online.hoprs.hoprs
MACOS_MIN=11.0

repo=$(cd "$(dirname "$0")/.." && pwd)

if [ $# -ne 1 ]; then
    echo "usage: $0 <path/to/hoprs.dmg>" >&2
    exit 2
fi
dmg=$1
version=$("$repo/scripts/workspace-version.sh")

failed=0
ok() { echo "OK    $1"; }
fail() {
    echo "FAIL  $1"
    failed=1
}
# stage <name> <command...>: runs the command, its output indented, OK or FAIL.
stage() {
    local name=$1
    shift
    echo "==> $name"
    if "$@" 2>&1 | sed 's/^/      /'; [ "${PIPESTATUS[0]}" -eq 0 ]; then
        ok "$name"
    else
        fail "$name"
    fi
}

if [ ! -f "$dmg" ]; then
    echo "FAIL  disk image exists: $dmg"
    exit 1
fi
ok "disk image exists: $dmg"
name=$(basename "$dmg")
if [ "$name" = "hoprs_${version}_macos_universal.dmg" ]; then
    ok "asset name $name"
else
    fail "asset name $name (expected hoprs_${version}_macos_universal.dmg)"
fi

stage "hdiutil verify" hdiutil verify "$dmg"

work=$(mktemp -d)
mnt=$work/mnt
mkdir "$mnt"
attached=0
cleanup() {
    if [ "$attached" -eq 1 ]; then
        hdiutil detach "$mnt" -quiet || hdiutil detach "$mnt" -force -quiet || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

if hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$mnt" "$dmg" >/dev/null; then
    attached=1
    ok "mount read-only"
else
    echo "FAIL  mount read-only"
    exit 1
fi

app=$mnt/hoprs.app
bin=$app/Contents/MacOS/hoprs
plist=$app/Contents/Info.plist

applications_link() {
    [ -L "$mnt/Applications" ] && [ "$(readlink "$mnt/Applications")" = /Applications ]
}
stage "Applications link to /Applications" applications_link
if [ ! -x "$bin" ]; then
    echo "FAIL  executable hoprs.app/Contents/MacOS/hoprs"
    exit 1
fi
ok "executable hoprs.app/Contents/MacOS/hoprs"

both_slices() {
    local archs
    archs=$(lipo -archs "$bin")
    echo "archs: $archs"
    [[ " $archs " == *" arm64 "* && " $archs " == *" x86_64 "* ]]
}
stage "universal binary (arm64 and x86_64)" both_slices

# minos of each slice, from its LC_BUILD_VERSION load command.
deployment_target() {
    local arch minos status=0
    for arch in arm64 x86_64; do
        minos=$(otool -arch "$arch" -l "$bin" |
            awk '/cmd LC_BUILD_VERSION/ { f = 1 } f && $1 == "minos" { print $2; exit }')
        echo "$arch: minos $minos"
        [ "$minos" = "$MACOS_MIN" ] || status=1
    done
    return "$status"
}
stage "each slice built for macOS $MACOS_MIN" deployment_target

stage "codesign --verify --deep --strict" codesign --verify --deep --strict --verbose=2 "$app"
ad_hoc() {
    local info
    info=$(codesign -dv "$app" 2>&1)
    echo "$info" | grep -E '^(Identifier|Signature)='
    grep -q '^Signature=adhoc$' <<<"$info"
}
stage "signature is ad hoc" ad_hoc

# plist_is <key> <expected>
plist_is() {
    local got
    got=$(/usr/libexec/PlistBuddy -c "Print :$1" "$plist")
    echo "$1 = $got"
    [ "$got" = "$2" ]
}
stage "CFBundleIdentifier = $BUNDLE_ID" plist_is CFBundleIdentifier "$BUNDLE_ID"
stage "CFBundleShortVersionString = $version" plist_is CFBundleShortVersionString "$version"
stage "LSMinimumSystemVersion = $MACOS_MIN" plist_is LSMinimumSystemVersion "$MACOS_MIN"
stage "CFBundleExecutable = hoprs" plist_is CFBundleExecutable hoprs
stage "CFBundlePackageType = APPL" plist_is CFBundlePackageType APPL

icon_file() {
    local icon
    icon=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIconFile" "$plist")
    echo "CFBundleIconFile = $icon"
    [ -f "$app/Contents/Resources/$icon" ] || [ -f "$app/Contents/Resources/$icon.icns" ] || return 1
    [ -f "$app/Contents/Resources/$icon" ] || icon=$icon.icns
    # iconutil refuses a malformed .icns; the iconset it writes lists what it read.
    iconutil -c iconset -o "$work/hoprs.iconset" "$app/Contents/Resources/$icon" || return 1
    ls "$work/hoprs.iconset"
}
stage "icon named by CFBundleIconFile present and readable" icon_file

# run_version [arch]: the binary's --version, in the given slice if one is named.
run_version() {
    local out
    if [ $# -eq 1 ]; then
        out=$(arch "-$1" "$bin" --version)
    else
        out=$("$bin" --version)
    fi
    echo "$out"
    [ "$out" = "hoprs $version" ]
}
stage "hoprs --version prints 'hoprs $version'" run_version
for slice in arm64 x86_64; do
    if arch "-$slice" /usr/bin/true 2>/dev/null; then
        stage "hoprs --version in the $slice slice" run_version "$slice"
    else
        echo "SKIP  hoprs --version in the $slice slice (this Mac cannot run $slice)"
    fi
done

# The builder's paths: HOPRS_BUILDER_HOME is a colon-separated list of prefixes that must
# not appear in the binary (default: this machine's $HOME), plus /Users/ itself. Run the
# check on the machine that built the image, or pass the list.
no_builder_paths() {
    local hits=0 pattern patterns=()
    IFS=: read -r -a patterns <<<"${HOPRS_BUILDER_HOME:-$HOME}"
    for pattern in /Users/ "${patterns[@]}"; do
        [ -n "$pattern" ] || continue
        if grep -qaF -- "$pattern" "$bin"; then
            echo "found '$pattern':"
            strings -a "$bin" | grep -F -- "$pattern" | head -n 5
            hits=1
        else
            echo "absent: $pattern"
        fi
    done
    return "$hits"
}
stage "no builder paths in the binary" no_builder_paths

if [ "$failed" -eq 0 ]; then
    echo "OK    all checks passed"
else
    echo "FAIL  some checks failed"
fi
exit "$failed"
