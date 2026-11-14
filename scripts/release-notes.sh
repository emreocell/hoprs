#!/usr/bin/env bash
# Prints the release notes of one version: the body of its `## <version>` section in
# CHANGELOG.md, up to the next `## ` heading, without the heading line and without
# leading or trailing blank lines. The heading may carry a suffix after a space
# (`## 1.0.0 — 2026-11-13`); `1.0.0` never matches `## 1.0.0-rc1` or `## 1.0.01`.
# CRLF line endings are accepted (and stripped from the output).
#
# Usage: scripts/release-notes.sh <version> [changelog]   (default: CHANGELOG.md at the repo root)
#        scripts/release-notes.sh --self-test
#
# Exits non-zero with a message if the section is missing or empty.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)

notes() {
    local version=$1 changelog=$2 body
    if [ ! -f "$changelog" ]; then
        echo "release-notes: $changelog not found" >&2
        return 1
    fi
    # Exit status 3: no such heading. Plain string comparisons, no regex: a version's
    # dots are not wildcards.
    body=$(awk -v want="## $version" '
        { sub(/\r$/, "") }
        /^## / {
            if (inside) exit
            if ($0 == want || index($0, want " ") == 1) { inside = 1; found = 1; next }
        }
        inside { print }
        END { if (!found) exit 3 }
    ' "$changelog") || {
        echo "release-notes: no '## $version' section in $changelog" >&2
        return 1
    }
    # Drop leading blank lines; $(...) already dropped the trailing ones.
    body=$(printf '%s\n' "$body" | sed '/[^[:space:]]/,$!d')
    if [ -z "$body" ]; then
        echo "release-notes: the '## $version' section in $changelog is empty" >&2
        return 1
    fi
    printf '%s\n' "$body"
}

self_test() {
    local dir fails=0
    dir=$(mktemp -d)
    # shellcheck disable=SC2064 # expand $dir now
    trap "rm -rf '$dir'" EXIT

    cat >"$dir/CHANGELOG.md" <<'EOF'
# Changelog

## 1.0.01 — 2026-12-02

Not 1.0.0 either.

## 1.0.0-rc1 — 2026-12-01

Release candidate, must not match 1.0.0.

## 1.0.0 — 2026-11-13

First release.

- Item one.
- Item two.

## 0.9.0

## 0.1.0 — 2026-01-01

Last section.
EOF
    printf '# Changelog\r\n\r\n## 2.0.0 — 2027-01-01\r\n\r\nWindows line.\r\n\r\n## 1.0.0\r\n\r\nOld.\r\n' \
        >"$dir/CRLF.md"

    expect_ok() { # name version file expected
        local got
        if got=$(notes "$2" "$3" 2>&1) && [ "$got" = "$4" ]; then
            echo "OK    $1"
        else
            echo "FAIL  $1: got [$got]"
            fails=$((fails + 1))
        fi
    }
    expect_fail() { # name version file
        local got
        if got=$(notes "$2" "$3" 2>&1); then
            echo "FAIL  $1: expected an error, got [$got]"
            fails=$((fails + 1))
        else
            echo "OK    $1 ($got)"
        fi
    }

    expect_ok "normal section, no heading, no other sections" 1.0.0 "$dir/CHANGELOG.md" \
        "$(printf 'First release.\n\n- Item one.\n- Item two.')"
    expect_ok "prefix collision: 1.0.0-rc1 is its own section" 1.0.0-rc1 "$dir/CHANGELOG.md" \
        "Release candidate, must not match 1.0.0."
    expect_ok "prefix collision: 1.0.01 is its own section" 1.0.01 "$dir/CHANGELOG.md" \
        "Not 1.0.0 either."
    expect_ok "last section, up to end of file" 0.1.0 "$dir/CHANGELOG.md" "Last section."
    expect_ok "CRLF line endings" 2.0.0 "$dir/CRLF.md" "Windows line."
    expect_ok "CRLF, last section" 1.0.0 "$dir/CRLF.md" "Old."
    expect_fail "missing version" 3.0.0 "$dir/CHANGELOG.md"
    expect_fail "prefix of a version is not a version" 1.0 "$dir/CHANGELOG.md"
    expect_fail "empty section" 0.9.0 "$dir/CHANGELOG.md"
    expect_fail "missing changelog file" 1.0.0 "$dir/nope.md"

    if [ "$fails" -ne 0 ]; then
        echo "release-notes self-test: $fails failed" >&2
        return 1
    fi
    echo "release-notes self-test: all passed"
}

case "${1:-}" in
    --self-test) self_test ;;
    "" | -*)
        echo "usage: $0 <version> [changelog] | --self-test" >&2
        exit 2
        ;;
    *) notes "$1" "${2:-$repo/CHANGELOG.md}" ;;
esac
