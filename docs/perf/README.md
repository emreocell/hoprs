# Performance (spec §9)

The v1 targets of the design spec (§9), how each one is measured, and the last numbers.
`scripts/perf.sh` runs every check; `.github/workflows/perf.yml` runs the same script on
`workflow_dispatch`, on every push to `main` and weekly, and puts the tables in the job
summary. It is a separate workflow, never a required pull-request check: shared runners
are noisy, so a miss fails the workflow without blocking a merge.

## The checks

| Target (spec §9) | Check | Criterion |
|---|---|---|
| Open in < 500 ms | `crates/hoprs-app/benches/startup.rs`: the release binary with `HOPRS_PROBE_STARTUP=1`, 5 runs under `xvfb-run`, tiny-skia renderer, fresh home; median | `startup_ms < 500` |
| Idle memory < 80 MB | The same probe, 2 s after the first frame, no session (`VmRSS`) | `rss_mb < 80` |
| 100k entries scroll at ≥ 60 fps | `crates/hoprs-app/benches/ui.rs` (criterion): `view()` of a tab whose pane lists 100 000 entries, plus the layout of the virtual list and the draw (tiny-skia recording the primitives; rasterization not included), at random offsets; p95 of 1 000 frames | p95 < 16 ms |
| SFTP 1 GB at 50 ms RTT ≥ 90% of `scp` | `crates/hoprs-transfer/benches/network.rs`: upload of a 1 GiB file by `TransferEngine` + `SftpFs` (4 workers) against `scp`, same latency, same file; median of 3 | hoprs ≥ 0.9 × `scp` (throughput) |
| 10k small files (SFTP and FTP) no worse than `lftp` | The same harness: 10 000 files of 4 KiB, 4 workers, against `lftp mirror -R --parallel=4`, over SFTP and over plain FTP, same latency; median of 3 | hoprs ≤ 1.0 × `lftp` (time) |
| Progress ≤ 10 Hz aggregated | `progress_rate_at_most_10hz` (`crates/hoprs-transfer/tests/engine_files.rs`): `Progress` events during 3 s of a 4-worker folder upload whose files keep finishing | ≤ 31 events in 3 s |

### Start-up probe

With `HOPRS_PROBE_STARTUP=1` the app starts as usual, then prints on stdout, and exits:

```
startup_ms=<n> rss_mb=<m>     # when the first frame has been drawn
idle_rss_mb=<m>               # 2 s later, idle, no session
```

`startup_ms` runs from the first line of `hoprs_app::run()` (which `main` calls; process
start-up before it is not counted) to the
first message iced handles after the first `RedrawRequested`, that is once that frame is
drawn and presented. The bench also reports the time from spawning the process to reading
that line (exec and dynamic loading included). Memory is `VmRSS` from `/proc/self/status`,
in MB (10^6 bytes). `hoprs_app::bench::parse_probe_line` reads the lines back.

### Network bench

`docker/compose.bench.yml` (project `hoprs-bench`, ports 2422 and 2421, passive ports
33000-33019) runs the images of the test environment with `NET_ADMIN`. Latency comes from
`tc netem` inside each container, on everything the container sends: a delay equal to the
whole round trip, one way (netem on `eth0` only shapes egress). The FTP server is on a
bridge network (netem cannot go on the host's interfaces, which the test FTP server
shares) and announces `pasv_address=127.0.0.1`, where its passive ports are published.
Uploads land on a tmpfs, so the server's disk is not what is measured.

The harness sets the latency per scenario (`tc qdisc replace`), empties the remote folder
before every run, checks afterwards that every byte or every file arrived, and alternates
which side goes first. Connecting counts in hoprs' time, as it does in the tools'. The
baselines authenticate with the fixture key `docker/fixtures/id_ed25519` (copied to a 0600
file); `lftp` reaches SFTP through `sftp:connect-program "ssh -a -x -i <key> ..."`. No
password prompt anywhere. The 1 GiB file is sparse.

The small files run at **10 ms** RTT by default, not 50 ms: at 50 ms the three small-file
comparisons (hoprs and `lftp`, three runs each) take about three hours.
`HOPRS_BENCH_SMALL_RTT_MS=50` reproduces the big file's latency.

Settings (`HOPRS_BENCH_*`, defaults match the compose file):

| Variable | Default | |
|---|---|---|
| `HOPRS_BENCH_NETWORK` | unset | `1` runs the bench; otherwise it skips (so `cargo bench -p hoprs-transfer` stays local) |
| `HOPRS_BENCH_COMPOSE` | `docker/compose.bench.yml` | |
| `HOPRS_BENCH_HOST` | `127.0.0.1` | |
| `HOPRS_BENCH_SSH_PORT`, `_SSH_USER`, `_SSH_KEY` | `2422`, `alice`, `docker/fixtures/id_ed25519` | |
| `HOPRS_BENCH_FTP_PORT`, `_FTP_USER`, `_FTP_PASSWORD` | `2421`, `frank`, `frank-pass` | |
| `HOPRS_BENCH_RTT_MS` | `50` | big file (and the containers' starting delay) |
| `HOPRS_BENCH_SMALL_RTT_MS` | `10` | small files |
| `HOPRS_BENCH_BIG_MIB` | `1024` | |
| `HOPRS_BENCH_SMALL_FILES` | `10000` | |
| `HOPRS_BENCH_RUNS` | `3` | |
| `HOPRS_BENCH_ONLY` | `sftp-big,sftp-small,ftp-small` | |

## Running locally

```sh
sudo apt install xvfb lftp openssh-client   # once
scripts/perf.sh                             # about 45 min; summary in target/perf/summary.md
```

The script starts `docker/compose.bench.yml` (beside the test environment, which it does
not touch) and stops it at the end (`HOPRS_PERF_KEEP=1` keeps it). Single checks:

```sh
cargo bench -p hoprs-app --bench ui
xvfb-run -a cargo bench -p hoprs-app --bench startup
docker compose -f docker/compose.bench.yml up -d --build --wait
HOPRS_BENCH_NETWORK=1 HOPRS_BENCH_SMALL_FILES=1000 HOPRS_BENCH_RUNS=1 \
  cargo bench -p hoprs-transfer --bench network
cargo test -p hoprs-transfer --test engine_files progress_rate_at_most_10hz
```

The bench exits with status 1 when a target is missed, 2 when it cannot run.

### Known gaps

A target listed in `HOPRS_BENCH_KNOWN_GAPS` (only `small_sftp` and `small_ftp` are accepted) is
still measured and printed, marked "known gap (see follow-up)", and does not fail the job;
every other target is fatal. `scripts/perf.sh` sets the list (see its header); a listed
target that passes again is printed as such and should leave the list.

## Last numbers

Local run of `scripts/perf.sh`, 2026-11-13, branch `plan-6-terminal-polish` at `10e0947`
(Ubuntu 26.04, 12 CPUs; Docker 29; `lftp` 4.9.2 and Xvfb from an Ubuntu 24.04 container,
`scp` OpenSSH 10.2 on the host, sshd OpenSSH 9.2 in the container). The CI runs of
`perf.yml` put theirs in the job summary.

| Target (spec §9) | Measured | Criterion | Result |
|---|---|---|---|
| Open < 500 ms | 93 ms (spawn to first frame: 96 ms) | `startup_ms < 500` | pass |
| Idle memory < 80 MB | 34.4 MB, 2 s after the first frame | `rss_mb < 80` | pass |
| 100k entries ≥ 60 fps | p95 0.68 ms per frame (max 1.14 ms), rasterization excluded | p95 < 16 ms | pass |
| SFTP 1 GiB, 50 ms RTT | hoprs 36.2 MiB/s, `scp` 36.4 MiB/s (28.2 s each) | ≥ 0.9 × `scp` | pass (1.00) |
| 10k × 4 KiB, SFTP, 10 ms RTT | hoprs 155.6 s, `lftp` 78.6 s | ≤ 1.0 × `lftp` | known gap (1.98) |
| 10k × 4 KiB, FTP, 10 ms RTT | hoprs 154.1 s, `lftp` 153.9 s | ≤ 1.0 × `lftp` | known gap (1.001) |
| Progress ≤ 10 Hz | `progress_rate_at_most_10hz` passes | ≤ 31 events in 3 s | pass |

Both 1 GiB uploads are bound by the same thing (the SSH channel window over a 50 ms round
trip), hence the identical times. FTP is at parity with `lftp`: the three runs of each
overlap (hoprs 154.0–154.7 s, `lftp` 153.9–154.9 s), but the median is 0.1% over, so the
target stays on the known-gap list until a run meets it.

History (same machine, same bench): SFTP small files 183.8 s → 155.6 s (2.22 → 1.98 ×
`lftp`); FTP small files 347.7 s → 154.1 s (2.24 → 1.00 × `lftp`).

### Fixed while measuring

- **Progress rate.** Every finished copy sent its final count at once: a folder of small
  files gave about 80 `Progress` events per second (238 in 3 s in the test). Final counts
  now go out at once only when the last event is an interval old; otherwise with the next
  tick, after the file's `Done` (ruling R-a). A file that runs again drops a pending one.
- **FTP per-file metadata.** The engine's `stat` of each file (`SIZE`, `STAT`, `MDTM`) and
  the `MFMT` after it queued on the control connection behind all four workers. They run
  on an idle transfer connection back in the pool for less than 2 s when one is free;
  such a connection is reused without a `NOOP`, and if it turns out dead its first
  command is retried once on a new connection.
- **No conflict check under a new folder.** A folder the run creates (missing before its
  `mkdir`, and its `mkdir` did not find it made meanwhile) is empty, so its files skip the
  conflict `stat` on their first attempt; a resumed, retried or recovered run checks
  again. Only while one item owns that folder: when another unfinished item writes to the
  same path, above it or below it, its files keep the check. **Accepted risk:** another
  client (outside this hoprs) creating a file in the new folder before the child is
  dispatched is not seen, and that file is overwritten.
- **Size-only check after a copy.** `RemoteFs::size`: FTP sends `SIZE` alone instead of
  `SIZE` + `STAT` + `MDTM`.

A missing destination's conflict check on FTP stays `SIZE` (550) then `STAT <path>`: for
a missing path `STAT` is one empty reply, not a directory scan, and it is what tells a
missing path from a directory or a symlink (rulings R10, R11). It no longer runs for the
files of a new folder.

### Round trips per file (upload into a new folder)

| | hoprs | `lftp mirror -R` |
|---|---|---|
| SFTP | 6: `lstat` in `open_write` (it refuses a FIFO; `open_write` adds an `fstat` only when continuing a file), `open`, the write acknowledgements, `close`, the size check (`lstat`), the time (`setstat`) | about 4 (open, write, close, attributes) |
| FTP | `PASV`, data connection, `STOR`, the final reply, the size check (`SIZE`); vsftpd has no `MFMT`, so no time | the same order of commands |

### Known gap: SFTP small files

hoprs still sends two more requests per file than `lftp` (the FIFO guard's `lstat` and
the size check's `lstat`), each a full round trip, so it takes about twice as long. Options
are in the follow-ups (`docs/superpowers/plans/2026-11-12-01-followups.md`): trust a fresh
`stat` in `open_write`, verify the size from the writer's acknowledged count, set the
time on the open handle.
