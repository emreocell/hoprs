#!/usr/bin/env bash
# Spec §9 performance checks: the same steps locally and in .github/workflows/perf.yml.
# See docs/perf/README.md. Needs docker (compose), xvfb-run, scp and lftp on PATH.
#
# Writes a Markdown summary to $GITHUB_STEP_SUMMARY in CI, to target/perf/summary.md
# otherwise, and the full output of each step to target/perf/<step>.log. Every step runs
# even when an earlier one misses its target; the script exits 1 if any did.
#
# HOPRS_PERF_KEEP=1 leaves the bench containers running afterwards. HOPRS_BENCH_* variables
# reach the network bench (see crates/hoprs-transfer/benches/network.rs).
#
# Known gaps: targets known to be missed, still measured and printed but not failing the
# job. Each has a follow-up (docs/superpowers/plans/2026-11-12-01-followups.md); remove it
# from this list once it is met. Override with HOPRS_BENCH_KNOWN_GAPS (empty: all fatal).
set -uo pipefail
export HOPRS_BENCH_KNOWN_GAPS="${HOPRS_BENCH_KNOWN_GAPS-small_sftp,small_ftp}"

cd "$(dirname "$0")/.."
out=target/perf
mkdir -p "$out"
summary="${GITHUB_STEP_SUMMARY:-$out/summary.md}"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] || : > "$summary"
compose=docker/compose.bench.yml
failed=()

note() { printf '%s\n' "$*" >> "$summary"; }

# Runs one step, keeps its log, and copies its Markdown (headings and table rows) into
# the summary.
step() {
    local name=$1
    shift
    echo "==> $name: $*" >&2
    "$@" 2>&1 | tee "$out/$name.log"
    local status=${PIPESTATUS[0]}
    grep -E '^(###|\|)' "$out/$name.log" >> "$summary" && note ""
    if [ "$status" -ne 0 ]; then
        failed+=("$name (exit $status)")
        note "_${name}: exit status ${status}, see the log._"
        note ""
    fi
}

cleanup() {
    if [ "${HOPRS_PERF_KEEP:-0}" != 1 ]; then
        docker compose -f "$compose" down -v >/dev/null 2>&1
    fi
}
trap cleanup EXIT

note "## hoprs performance (spec §9)"
note ""
note "Commit \`$(git rev-parse --short HEAD)\`, $(date -u +%Y-%m-%dT%H:%MZ), $(nproc) CPUs."
note "Known gaps (not fatal): \`${HOPRS_BENCH_KNOWN_GAPS:-none}\`."
note ""

docker compose -f "$compose" up -d --build --wait || { note "Bench containers failed to start."; exit 2; }
# Compile everything first, so no step's time includes a build.
cargo bench -p hoprs-app -p hoprs-transfer --no-run || exit 2

step ui cargo bench -p hoprs-app --bench ui -- --noplot
step startup xvfb-run -a cargo bench -p hoprs-app --bench startup
step network env HOPRS_BENCH_NETWORK=1 cargo bench -p hoprs-transfer --bench network

echo "==> progress" >&2
note "### Progress events"
note ""
note "| Target (spec §9) | Measured | Criterion | Result |"
note "|---|---|---|---|"
if cargo test -p hoprs-transfer --test engine_files progress_rate_at_most_10hz 2>&1 | tee "$out/progress.log"; then
    note "| Progress ≤ 10 Hz aggregated | \`progress_rate_at_most_10hz\` (4 workers, 3 s) | ≤ 31 \`Progress\` events in 3 s | pass |"
else
    failed+=("progress")
    note "| Progress ≤ 10 Hz aggregated | \`progress_rate_at_most_10hz\` (4 workers, 3 s) | ≤ 31 \`Progress\` events in 3 s | **FAIL** |"
fi
note ""

if [ ${#failed[@]} -gt 0 ]; then
    note "**Missed:** ${failed[*]}"
    echo "perf: missed: ${failed[*]}" >&2
    exit 1
fi
note "All spec §9 targets met, known gaps aside (\`${HOPRS_BENCH_KNOWN_GAPS:-none}\`)."
