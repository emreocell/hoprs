#!/usr/bin/env bash
# Runs inside the clean Ubuntu 24.04 container (docker/deb-check/Dockerfile), started by
# scripts/check-deb.sh with the package mounted read-only. Usage: hoprs-deb-check <deb>
#
# Every stage prints OK or FAIL; the script exits 1 if any stage failed. Stages that make
# no sense after an earlier failure (nothing installed, say) are skipped, not counted OK.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

APP_ID=io.github.engviniciusbs.hoprs
SIZES=(16 24 32 48 64 128 256 512)
failed=()

ok() { printf 'OK    %s\n' "$1"; }
fail() {
    printf 'FAIL  %s\n' "$1"
    failed+=("$1")
}
# stage <name> <command...>: runs the command, its output indented, and reports it.
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
finish() {
    echo
    if [ ${#failed[@]} -eq 0 ]; then
        echo "deb check: all stages OK"
        exit 0
    fi
    echo "deb check: ${#failed[@]} stage(s) failed: ${failed[*]}"
    exit 1
}

# The crate version (what `hoprs --version` prints) of a Debian version cargo-deb made:
# the Debian revision (`-1`) dropped, and the `~` cargo-deb puts in a prerelease
# (`1.0.0-rc1` -> `1.0.0~rc1-1`, so it sorts before 1.0.0) mapped back to `-`. Semver
# has no `~`, so every `~` came from cargo-deb.
cargo_version() {
    local upstream=${1%-*}
    printf '%s\n' "${upstream//\~/-}"
}

# --self-test: checks cargo_version against the versions cargo-deb 3.8.0 produces.
self_test() {
    local fails=0 deb want got
    while read -r deb want; do
        got=$(cargo_version "$deb")
        if [ "$got" = "$want" ]; then
            echo "ok    $deb -> $got"
        else
            echo "FAIL  $deb -> $got, want $want"
            fails=$((fails + 1))
        fi
    done <<'CASES'
1.0.0-1 1.0.0
1.2.3-2 1.2.3
1.0.0~rc1-1 1.0.0-rc1
1.0.0~rc.1+build.5-1 1.0.0-rc.1+build.5
1.0.0~beta-2-1 1.0.0-beta-2
1.0.0-rc-1 1.0.0-rc
CASES
    if [ "$fails" -ne 0 ]; then
        echo "deb check self-test: $fails failed"
        exit 1
    fi
    echo "deb check self-test: all passed"
    exit 0
}
if [ "${1:-}" = --self-test ]; then
    self_test
fi

deb_src=${1:?usage: hoprs-deb-check <deb> | --self-test}
# apt installs local files by path; a copy keeps it out of the read-only mount.
deb=/tmp/$(basename "$deb_src")
cp "$deb_src" "$deb"
version=$(dpkg-deb -f "$deb" Version)
upstream=$(cargo_version "$version")

# Files the package must install, and remove again.
installed_files=(
    /usr/bin/hoprs
    "/usr/share/applications/$APP_ID.desktop"
    "/usr/share/metainfo/$APP_ID.metainfo.xml"
    "/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
)
for n in "${SIZES[@]}"; do
    installed_files+=("/usr/share/icons/hicolor/${n}x${n}/apps/$APP_ID.png")
done
doc_files=(/usr/share/doc/hoprs/LICENSE-MIT /usr/share/doc/hoprs/LICENSE-APACHE
    /usr/share/doc/hoprs/copyright /usr/share/doc/hoprs/fonts/OFL-Inter.txt
    /usr/share/doc/hoprs/fonts/OFL-JetBrainsMono.txt /usr/share/doc/hoprs/fonts/LICENSE-Phosphor.txt)

# The package's exact contents, files only, as "mode path": the binary 755, the rest 644.
expected_contents() {
    local f
    for f in "${installed_files[@]}" "${doc_files[@]}"; do
        if [ "$f" = /usr/bin/hoprs ]; then echo "-rwxr-xr-x .$f"; else echo "-rw-r--r-- .$f"; fi
    done | sort
}

# User data hoprs must never touch on remove or purge (Review Focus 3).
user_files=("$HOME/.config/hoprs/sites.toml" "$HOME/.local/share/hoprs/x")
marker="user data, must survive uninstall"

all_exist() {
    local missing=0 f
    for f in "$@"; do
        if [ -e "$f" ]; then echo "present: $f"; else echo "missing: $f"; missing=1; fi
    done
    return "$missing"
}
none_exist() {
    local left=0 f
    for f in "$@"; do
        if [ -e "$f" ]; then echo "still there: $f"; left=1; else echo "gone: $f"; fi
    done
    return "$left"
}
user_data_intact() {
    local bad=0 f
    for f in "${user_files[@]}"; do
        if [ "$(cat "$f" 2>/dev/null)" = "$marker" ]; then echo "intact: $f"; else echo "lost: $f"; bad=1; fi
    done
    return "$bad"
}

stage "apt update" apt-get update -q
# Simulated with Recommends, so the Recommends and Suggests at least resolve on Ubuntu.
stage "dependencies resolve (with recommends, simulated)" apt-get install -s -q "$deb"
# Without Recommends: Depends alone must be enough to run.
stage "install (depends only)" apt-get install -y -q --no-install-recommends "$deb"
if ! dpkg -s hoprs >/dev/null 2>&1; then
    fail "hoprs is installed"
    finish
fi
# What hoprs and its Depends brought in, before any check tool adds libraries.
dpkg-query -W -f='${Package}\n' | sed 's/:.*//' | sort -u > /tmp/packages-with-hoprs

check_version() {
    local out
    # No display: proves --version exits before any window.
    out=$(env -u DISPLAY -u WAYLAND_DISPLAY timeout 10 hoprs --version)
    echo "$out"
    [ "$out" = "hoprs $upstream" ]
}
check_contents() {
    local actual
    actual=$(dpkg-deb -c "$deb" | awk '$1 !~ /^d/ {print $1, $NF}' | sort)
    if diff <(expected_contents) <(echo "$actual"); then
        echo "$(wc -l <<< "$actual") files, modes as expected"
    else
        return 1
    fi
    # Directories: 755, all of them.
    ! dpkg-deb -c "$deb" | awk '$1 ~ /^d/ && $1 != "drwxr-xr-x" {print "bad mode:", $1, $NF; bad=1}
        END {exit !bad}'
}
stage "package contents: exact file list and modes" check_contents

# The binary must not carry the builder's paths (panic locations, debug info): no /home/
# at all, and none of the builder's own prefixes (HOPRS_BUILDER_HOME from check-deb.sh, a
# colon-separated list: the builder's $HOME, the container build's paths, ...).
no_builder_paths() {
    local hits=0 pattern patterns=()
    IFS=: read -r -a patterns <<<"${HOPRS_BUILDER_HOME:-}"
    for pattern in /home/ "${patterns[@]}"; do
        [ -n "$pattern" ] || continue
        if grep -qaF -- "$pattern" /usr/bin/hoprs; then
            echo "found '$pattern':"
            tr -c '[:print:]' '\n' < /usr/bin/hoprs | grep -F -- "$pattern" | head -n 5
            hits=1
        else
            echo "absent: $pattern"
        fi
    done
    return "$hits"
}

stage "hoprs --version prints 'hoprs $upstream'" check_version
stage "installed files present" all_exist "${installed_files[@]}" "${doc_files[@]}"
stage "no builder paths in the binary" no_builder_paths

stage "install check tools (xvfb, desktop-file-utils, appstream)" \
    apt-get install -y -q --no-install-recommends xvfb xauth desktop-file-utils appstream
stage "desktop-file-validate" desktop-file-validate "/usr/share/applications/$APP_ID.desktop"
check_metainfo() {
    local status=0
    appstreamcli validate --no-net "/usr/share/metainfo/$APP_ID.metainfo.xml" || status=$?
    # 3: warnings only (url-homepage-missing), accepted.
    [ "$status" -eq 0 ] || [ "$status" -eq 3 ]
}
stage "appstreamcli validate (0 or 3)" check_metainfo

# probe <name> [VAR=value...]: starts the installed binary under Xvfb with the start-up
# probe, which prints `startup_ms=...` after the first frame drawn and exits. The dynamic
# loader logs every library hoprs loads, dlopen'ed ones included, to /tmp/ld-<name>.*
probe() {
    local name=$1 out
    shift
    rm -f "/tmp/ld-$name".*
    out=$(timeout 60 xvfb-run -a env -u WAYLAND_DISPLAY "$@" HOPRS_PROBE_STARTUP=1 \
        LD_DEBUG=libs LD_DEBUG_OUTPUT="/tmp/ld-$name" hoprs)
    echo "$out"
    printf '%s\n' "$out" > "/tmp/probe-$name.out"
    grep -q '^startup_ms=' <<< "$out"
}
stage "probe under xvfb-run (tiny-skia)" probe tiny-skia ICED_BACKEND=tiny-skia
# The renderer a user gets by default (no ICED_BACKEND). Without a GPU render node, as
# here, hoprs picks tiny-skia itself (crates/hoprs-app/src/renderer.rs); otherwise iced
# would run wgpu on Mesa's software adapter, at about 165 MB.
stage "probe under xvfb-run (default renderer)" probe default
# Spec §9: under 80 MB. The memory reading of the default run above.
default_rss_under_80() {
    local rss
    rss=$(sed -n 's/^startup_ms=[0-9]* rss_mb=\([0-9.]*\)$/\1/p' /tmp/probe-default.out)
    echo "rss_mb=${rss:-none}"
    [ -n "$rss" ] && awk -v rss="$rss" 'BEGIN {exit !(rss < 80)}'
}
stage "default renderer: rss_mb < 80" default_rss_under_80

# Every library the probes loaded must come from a package hoprs' Depends brought in, not
# from one the check tools installed afterwards. This proves everything loaded comes from
# Depends; it cannot prove everything needed is in Depends (a path these probes never
# take, Wayland for one, may load more).
# The package that owns a file, trying the merged-/usr spellings of its path.
owner() {
    local path
    for path in "$1" "$(realpath "$1")" "/usr${1#/usr}" "${1#/usr}"; do
        # `dpkg -S` also prints diversion lines; only "package: path" ones count.
        dpkg -S "$path" 2>/dev/null | grep -v '^diversion' | sed -n '1s/[:,].*//p' | grep . \
            && return 0
    done
    return 0
}
loaded_libs_covered() {
    local libs lib pkg bad=0
    libs=$(sed -n 's/.*calling init: \(\/.*\)$/\1/p' /tmp/ld-*.* | sort -u)
    [ -n "$libs" ] || { echo "no loader log"; return 1; }
    for lib in $libs; do
        pkg=$(owner "$lib")
        if [ -z "$pkg" ]; then
            echo "no package: $lib"
            bad=1
        elif grep -qx "$pkg" /tmp/packages-with-hoprs; then
            echo "ok: $lib ($pkg)"
        else
            echo "NOT FROM DEPENDS: $lib ($pkg)"
            bad=1
        fi
    done
    return "$bad"
}
stage "every loaded library comes from Depends" loaded_libs_covered

make_user_data() {
    local f
    for f in "${user_files[@]}"; do
        mkdir -p "$(dirname "$f")"
        printf '%s\n' "$marker" > "$f"
        echo "created: $f"
    done
}
stage "create user data" make_user_data

stage "apt-get remove" apt-get remove -y -q hoprs
stage "installed files and docs gone after remove" none_exist "${installed_files[@]}" \
    "${doc_files[@]}" /usr/share/doc/hoprs/fonts /usr/share/doc/hoprs
stage "user data intact after remove" user_data_intact
# dpkg, not apt: after a remove that left no conffiles, apt no longer knows the package.
stage "purge" dpkg --purge hoprs
not_installed() { ! dpkg -s hoprs 2>/dev/null | grep -q '^Status: install'; }
stage "hoprs no longer installed" not_installed
stage "nothing left after purge" none_exist "${installed_files[@]}" "${doc_files[@]}" \
    /usr/share/doc/hoprs
stage "user data intact after purge" user_data_intact

finish
